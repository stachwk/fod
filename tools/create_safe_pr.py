#!/usr/bin/env python3
"""Create a GitHub pull request only after local-text privacy validation.

The wrapper validates the PR title and body file before invoking `gh pr create`.
It passes the body to GitHub by file path and never prints the body contents.
"""

from __future__ import annotations

import argparse
import subprocess
import sys
from pathlib import Path

from check_github_text_privacy import scan_text


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        description=(
            "Validate a pull-request title and body for likely local metadata, "
            "then create the PR with GitHub CLI."
        )
    )
    parser.add_argument("--title", required=True, help="pull-request title")
    parser.add_argument(
        "--body-file",
        required=True,
        type=Path,
        help="UTF-8 Markdown file containing the pull-request body",
    )
    parser.add_argument("--base", help="base branch passed to gh pr create")
    parser.add_argument("--head", help="head branch passed to gh pr create")
    parser.add_argument("--repo", help="repository passed to gh pr create")

    publication_mode = parser.add_mutually_exclusive_group()
    publication_mode.add_argument(
        "--draft",
        dest="draft",
        action="store_true",
        default=True,
        help="create a draft pull request (default)",
    )
    publication_mode.add_argument(
        "--ready",
        dest="draft",
        action="store_false",
        help="create a pull request ready for review",
    )

    parser.add_argument(
        "--dry-run",
        action="store_true",
        help="validate the text without invoking GitHub CLI",
    )
    return parser


def _read_body(path: Path) -> str:
    try:
        return path.read_text(encoding="utf-8")
    except OSError as error:
        raise SystemExit("unable to read PR body file") from error


def _validate_publication_text(title: str, body: str) -> bool:
    findings = scan_text(f"{title}\n{body}")
    if not findings:
        return True

    print(
        f"safe PR creation blocked: {len(findings)} potential local-data reference(s)",
        file=sys.stderr,
    )
    for finding in findings:
        print(f"- line {finding.line}: {finding.rule}", file=sys.stderr)
    print(
        "replace local values with aggregate results or synthetic placeholders",
        file=sys.stderr,
    )
    return False


def _gh_command(args: argparse.Namespace) -> list[str]:
    command = [
        "gh",
        "pr",
        "create",
        "--title",
        args.title,
        "--body-file",
        str(args.body_file),
    ]
    if args.base:
        command.extend(["--base", args.base])
    if args.head:
        command.extend(["--head", args.head])
    if args.repo:
        command.extend(["--repo", args.repo])
    if args.draft:
        command.append("--draft")
    return command


def main(argv: list[str] | None = None) -> int:
    args = build_parser().parse_args(argv)
    body = _read_body(args.body_file)

    if not _validate_publication_text(args.title, body):
        return 1

    if args.dry_run:
        print("safe PR text passed; GitHub command not executed")
        return 0

    try:
        result = subprocess.run(_gh_command(args), check=False)
    except FileNotFoundError:
        print("unable to run GitHub CLI: gh was not found", file=sys.stderr)
        return 127
    return result.returncode


if __name__ == "__main__":
    raise SystemExit(main())
