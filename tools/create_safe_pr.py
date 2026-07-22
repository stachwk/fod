#!/usr/bin/env python3
"""Create a GitHub pull request only after privacy and branch validation.

The wrapper validates the PR title and body before invoking `gh pr create`.
It passes the body to GitHub by file path and never prints the body contents.
"""

from __future__ import annotations

import argparse
import subprocess
import sys
from pathlib import Path

from check_github_text_privacy import scan_text


DEFAULT_BODY_FILE = Path(".git/fod-private/pr-body.md")
DEFAULT_TEMPLATE_FILE = Path(".github/PULL_REQUEST_TEMPLATE.md")


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        description=(
            "Validate pull-request text and branch state, then create the PR "
            "with GitHub CLI."
        )
    )
    parser.add_argument(
        "--title",
        help="pull-request title; required unless --prepare-body is used",
    )
    parser.add_argument(
        "--body-file",
        type=Path,
        default=DEFAULT_BODY_FILE,
        help=(
            "UTF-8 Markdown file containing the pull-request body "
            "(default: .git/fod-private/pr-body.md)"
        ),
    )
    parser.add_argument(
        "--prepare-body",
        action="store_true",
        help="copy the repository PR template to the body file when it is absent",
    )
    parser.add_argument(
        "--base",
        default="main",
        help="base branch passed to gh pr create (default: main)",
    )
    parser.add_argument("--head", help="head branch; defaults to the current branch")
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
        help="validate the text without checking branch state or invoking GitHub CLI",
    )
    return parser


def _read_body(path: Path) -> str:
    try:
        return path.read_text(encoding="utf-8")
    except OSError as error:
        raise SystemExit("unable to read PR body file") from error


def _prepare_body(path: Path) -> int:
    if path.exists():
        print("safe PR body already exists; not overwritten")
        return 0

    try:
        template = DEFAULT_TEMPLATE_FILE.read_text(encoding="utf-8")
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(template, encoding="utf-8")
    except OSError:
        print("unable to prepare safe PR body file", file=sys.stderr)
        return 1

    print("safe PR body prepared")
    return 0


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


def _git_stdout(*arguments: str) -> str | None:
    try:
        result = subprocess.run(
            ["git", *arguments],
            text=True,
            capture_output=True,
            check=False,
        )
    except FileNotFoundError:
        print("safe PR creation blocked: git was not found", file=sys.stderr)
        return None

    if result.returncode != 0:
        print("safe PR creation blocked: git preflight failed", file=sys.stderr)
        return None
    return result.stdout.strip()


def _validated_head(explicit_head: str | None, base: str) -> str | None:
    head = explicit_head or _git_stdout("branch", "--show-current")
    if head is None:
        return None
    if not head:
        print(
            "safe PR creation blocked: unable to determine the current branch",
            file=sys.stderr,
        )
        return None
    if head == base:
        print(
            "safe PR creation blocked: head branch matches the base branch",
            file=sys.stderr,
        )
        return None

    ahead_text = _git_stdout("rev-list", "--count", f"{base}..{head}")
    if ahead_text is None:
        return None
    try:
        ahead = int(ahead_text)
    except ValueError:
        print(
            "safe PR creation blocked: invalid git preflight result",
            file=sys.stderr,
        )
        return None
    if ahead < 1:
        print(
            "safe PR creation blocked: head branch has no commits ahead of base",
            file=sys.stderr,
        )
        return None
    return head


def _gh_command(args: argparse.Namespace, head: str) -> list[str]:
    command = [
        "gh",
        "pr",
        "create",
        "--title",
        args.title,
        "--body-file",
        str(args.body_file),
        "--base",
        args.base,
        "--head",
        head,
    ]
    if args.repo:
        command.extend(["--repo", args.repo])
    if args.draft:
        command.append("--draft")
    return command


def main(argv: list[str] | None = None) -> int:
    parser = build_parser()
    args = parser.parse_args(argv)

    if args.prepare_body:
        return _prepare_body(args.body_file)
    if not args.title:
        parser.error("--title is required unless --prepare-body is used")

    body = _read_body(args.body_file)
    if not _validate_publication_text(args.title, body):
        return 1

    if args.dry_run:
        print("safe PR text passed; GitHub command not executed")
        return 0

    head = _validated_head(args.head, args.base)
    if head is None:
        return 1

    try:
        result = subprocess.run(_gh_command(args, head), check=False)
    except FileNotFoundError:
        print("unable to run GitHub CLI: gh was not found", file=sys.stderr)
        return 127
    return result.returncode


if __name__ == "__main__":
    raise SystemExit(main())
