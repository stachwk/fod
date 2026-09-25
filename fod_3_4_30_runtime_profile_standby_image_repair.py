#!/usr/bin/env python
# -*- coding: utf-8 -*-

import argparse
import sys
from pathlib import Path


def print_err(message):
    print(f"stderr: {message}", file=sys.stderr)


def main():
    parser = argparse.ArgumentParser(
        description="Repair the partially applied FOD 3.4.30 runtime-profile standby image patch."
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

    required_markers = (
        "def _docker_container_image(",
        "primary_image = _docker_container_image()",
        "primary_image,",
    )
    missing = [marker for marker in required_markers if marker not in text]
    if missing:
        print_err(f"standby image patch is not in the expected partially applied state: missing={missing}")
        return 4

    broken = """            raise AssertionError(
                "recovery standby exited before becoming ready:
"
                f"state={state_text}
"
                f"stdout:
{logs.stdout}
"
                f"stderr:
{logs.stderr}"
            )
"""

    fixed = """            raise AssertionError(
                "recovery standby exited before becoming ready:\\n"
                f"state={state_text}\\n"
                f"stdout:\\n{logs.stdout}\\n"
                f"stderr:\\n{logs.stderr}"
            )
"""

    if broken in text:
        new_text = text.replace(broken, fixed, 1)
    elif fixed in text:
        if args.verbose:
            print_err("standby image string-literal repair already applied")
        print("OK: repair already present")
        return 0
    else:
        print_err("expected broken or fixed recovery AssertionError block was not found")
        return 5

    try:
        compile(new_text, str(test_path), "exec")
    except SyntaxError as exc:
        print_err(f"repaired file still has syntax error: {exc}")
        return 6

    test_path.write_text(new_text, encoding="utf-8")

    if args.verbose:
        print_err("repaired malformed recovery standby AssertionError string literals")

    print("OK: repaired test_runtime_profile.py syntax")
    print("OK: standby image patch remains applied")
    print("OK: version remains 3.4.30")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
