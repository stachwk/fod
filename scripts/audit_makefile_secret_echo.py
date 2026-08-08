#!/usr/bin/env python3
from __future__ import annotations

import argparse
from pathlib import Path

SENSITIVE_EXPANSIONS = (
    "$(POSTGRES_PASSWORD)",
    "$(POSTGRES_PASSWORD_BASE)",
    "$(FOD_PG_PASSWORD)",
    "$(FOD_REMOTE_PG_PASSWORD)",
    "$(QNAP_PG_PASSWORD)",
    "$(FOD_SCHEMA_ADMIN_PASSWORD)",
    "$(FOD_CHANGE_PASSWORD)",
    "$(FOD_REMOTE_PG_ENV)",
)


def logical_recipe_commands(lines: list[str]):
    index = 0
    while index < len(lines):
        if not lines[index].startswith("\t"):
            index += 1
            continue
        start = index
        block = [lines[index]]
        while block[-1].rstrip("\n").endswith("\\") and index + 1 < len(lines):
            index += 1
            block.append(lines[index])
        yield start + 1, block
        index += 1


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--makefile", default="Makefile")
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()

    path = Path(args.makefile)
    lines = path.read_text(encoding="utf-8").splitlines(keepends=True)
    problems: list[str] = []
    checked = 0

    for line_number, block in logical_recipe_commands(lines):
        command = "".join(block)
        if not any(token in command for token in SENSITIVE_EXPANSIONS):
            continue
        checked += 1
        first = block[0][1:] if block[0].startswith("\t") else block[0]
        if not first.startswith("@"):
            tokens = [token for token in SENSITIVE_EXPANSIONS if token in command]
            problems.append(
                f"{path}:{line_number}: secret-bearing recipe is echoable: "
                + ", ".join(tokens)
            )

    print(
        f"Makefile secret-echo audit: checked_secret_commands={checked} "
        f"problems={len(problems)}"
    )
    for problem in problems:
        print(problem)

    if args.check and problems:
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
