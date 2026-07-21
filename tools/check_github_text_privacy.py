#!/usr/bin/env python3
"""Reject likely local catalogue metadata before text is published to GitHub.

The checker reports only rule names and line numbers. It never prints the
matched value, so running it does not copy sensitive paths into another log.
"""

from __future__ import annotations

import argparse
import re
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import Iterable


@dataclass(frozen=True)
class Finding:
    line: int
    rule: str
    description: str


@dataclass(frozen=True)
class Rule:
    name: str
    description: str
    pattern: re.Pattern[str]


RULES: tuple[Rule, ...] = (
    Rule(
        "unix-user-path",
        "possible user home, media, mount, or runtime path",
        re.compile(r"(?i)(?:^|[\s='\"])/(?:home|media|mnt|Users|run/user)/[^\s'\"`]+"),
    ),
    Rule(
        "home-relative-path",
        "possible home-relative path",
        re.compile(r"(?:^|[\s='\"])(?:~/|\$HOME/)[^\s'\"`]+"),
    ),
    Rule(
        "windows-user-path",
        "possible Windows user-profile path",
        re.compile(r"(?i)\b[A-Z]:\\Users\\[^\s\"']+"),
    ),
    Rule(
        "shell-prompt-identity",
        "possible username and hostname copied from a shell prompt",
        re.compile(r"(?m)^[^\s@]+@[^\s:]+:[^\n]*[$#]\s"),
    ),
    Rule(
        "raw-indexer-progress",
        "raw FOD scan or hash progress output",
        re.compile(r"(?m)^FOD indexer (?:scan|hash) progress:"),
    ),
    Rule(
        "current-file-field",
        "current file name or path from a local operation",
        re.compile(r"\bcurrent=[^\s]+"),
    ),
    Rule(
        "registered-source-log",
        "raw source registration output",
        re.compile(r"(?m)^Registered source\s+"),
    ),
    Rule(
        "catalogue-hash-field",
        "full catalogue hash copied from local output",
        re.compile(r'"full_hash_hex"\s*:\s*"[0-9a-fA-F]{32,}"'),
    ),
    Rule(
        "catalogue-timestamp-field",
        "catalogue timestamp copied from local output",
        re.compile(r'"(?:created_at|updated_at)"\s*:\s*"[^\"]+"'),
    ),
)


SAFE_SYNTHETIC_PREFIXES: tuple[str, ...] = (
    "/tmp/fod-indexer-test-",
    "/tmp/fod-indexer-smoke-",
    "/tmp/fod-indexer-fixture-",
)


def _line_number(text: str, offset: int) -> int:
    return text.count("\n", 0, offset) + 1


def _is_safe_synthetic_match(value: str) -> bool:
    return any(prefix in value for prefix in SAFE_SYNTHETIC_PREFIXES)


def scan_text(text: str) -> list[Finding]:
    findings: list[Finding] = []
    seen: set[tuple[int, str]] = set()

    for rule in RULES:
        for match in rule.pattern.finditer(text):
            matched = match.group(0)
            if _is_safe_synthetic_match(matched):
                continue
            key = (_line_number(text, match.start()), rule.name)
            if key in seen:
                continue
            seen.add(key)
            findings.append(
                Finding(
                    line=key[0],
                    rule=rule.name,
                    description=rule.description,
                )
            )

    findings.sort(key=lambda item: (item.line, item.rule))
    return findings


def _read_inputs(paths: Iterable[str]) -> str:
    selected = list(paths)
    if not selected or selected == ["-"]:
        return sys.stdin.read()

    chunks: list[str] = []
    for raw_path in selected:
        path = Path(raw_path)
        try:
            chunks.append(path.read_text(encoding="utf-8"))
        except OSError as error:
            raise SystemExit(f"unable to read input file: {path}: {error}") from error
    return "\n".join(chunks)


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        description=(
            "Check a prepared GitHub description or comment for likely local "
            "file, directory, host, or catalogue metadata."
        )
    )
    parser.add_argument(
        "paths",
        nargs="*",
        metavar="FILE",
        help="UTF-8 text file to check; omit or use '-' to read stdin",
    )
    parser.add_argument(
        "--quiet",
        action="store_true",
        help="print nothing when the check succeeds",
    )
    return parser


def main(argv: list[str] | None = None) -> int:
    args = build_parser().parse_args(argv)
    text = _read_inputs(args.paths)
    findings = scan_text(text)

    if findings:
        print(
            f"privacy check failed: {len(findings)} potential local-data reference(s)",
            file=sys.stderr,
        )
        for finding in findings:
            print(
                f"- line {finding.line}: {finding.rule} ({finding.description})",
                file=sys.stderr,
            )
        print(
            "replace local values with aggregate results or synthetic placeholders",
            file=sys.stderr,
        )
        return 1

    if not args.quiet:
        print("privacy check passed: no likely local-data references found")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
