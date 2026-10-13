#!/usr/bin/env python3
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1
"""C10-B2: conservative source locations for SQL mutations and execution sinks.

This is a static candidate inventory, not an SQL parser or a proof that an
operation is safe or fenced. Every finding requires transaction-level review.
No database connection is made and no production data is accessed.
"""

from __future__ import annotations

import argparse
import json
import re
from collections import Counter
from pathlib import Path

RUST_DIRS = (
    "rust_fuse/src",
    "rust_hotpath/src",
    "rust_indexer/src",
    "rust_libfod/src",
    "rust_mkfs/src",
    "rust_monitor/src",
    "rust_runtime/src",
)
SQL_DIRS = ("migrations",)
AUX_SOURCE_DIRS = ("scripts", "docker")
AUX_EXTENSIONS = frozenset((".py", ".sh", ".sql"))
OPERATION = re.compile(
    r"\b(?P<operation>"
    r"INSERT\s+INTO|UPDATE|DELETE\s+FROM|TRUNCATE(?:\s+TABLE)?|"
    r"CREATE\s+(?:OR\s+REPLACE\s+)?(?:TABLE|INDEX|SCHEMA|ROLE|DATABASE|EXTENSION|VIEW|FUNCTION|TRIGGER|SEQUENCE)|"
    r"ALTER\s+(?:TABLE|SCHEMA|ROLE|DATABASE|FUNCTION|SEQUENCE)|"
    r"DROP\s+(?:TABLE|INDEX|SCHEMA|ROLE|DATABASE|EXTENSION|VIEW|FUNCTION|TRIGGER|SEQUENCE)|"
    r"GRANT|REVOKE"
    r")\s+(?:IF\s+(?:NOT\s+)?EXISTS\s+)?"
    r"(?P<object>(?:[A-Za-z_][\w]*\.)?[A-Za-z_][\w]*)?",
    re.IGNORECASE,
)
EXECUTION_SINK = re.compile(
    r"\b(?P<sink>"
    r"PQexec(?:Params)?|exec_command_params|exec_params|exec_raw|"
    r"query_rows_text|query_scalar_text|"
    r"(?:repo|conn|self)\s*\.\s*exec|"
    r"(?:cursor|connection|conn)\s*\.\s*execute"
    r")\s*\(",
)
FUNCTION = re.compile(
    r"^\s*(?:(?:pub(?:\([^)]*\))?|unsafe|async)\s+)*fn\s+([A-Za-z_]\w*)"
)
GUARD_HINTS = (
    "validate_write_persistence_fence_on_conn",
    "reject_unfenced_write_during_active_lease_on_conn",
    "try_advisory_xact_lock_text_on_conn",
    "transactional_replayable",
    "transactional_replay_confirmed",
)


def discover(root: Path) -> list[Path]:
    paths: set[Path] = set()
    for dirname in RUST_DIRS:
        folder = root / dirname
        if folder.is_dir():
            paths.update(p for p in folder.rglob("*.rs") if p.is_file())
    for dirname in SQL_DIRS:
        folder = root / dirname
        if folder.is_dir():
            paths.update(p for p in folder.rglob("*.sql") if p.is_file())
    for dirname in AUX_SOURCE_DIRS:
        folder = root / dirname
        if folder.is_dir():
            paths.update(
                p for p in folder.rglob("*")
                if p.is_file() and p.suffix in AUX_EXTENSIONS
            )
    return sorted(paths)


def _category(path: str, operation: str, relation: str) -> str:
    if path.startswith("migrations/") or path.startswith("rust_mkfs/"):
        return "schema_admin"
    text = (operation + " " + relation).lower()
    if "lease" in text or "session" in text:
        return "lease_or_session"
    if any(name in text for name in ("files", "directories", "data_blocks", "data_objects")):
        return "filesystem_state"
    if "index_" in text or "snapshot" in text:
        return "index_catalog"
    return "manual_review"


def _symbol_lines(lines: list[str]) -> list[str]:
    current = "<module>"
    symbols = []
    for line in lines:
        matched = FUNCTION.match(line)
        if matched:
            current = matched.group(1)
        symbols.append(current)
    # Rust functions can span an impl; names are navigation hints, not scopes.
    return symbols


def inventory_file(path: Path, root: Path) -> list[dict[str, object]]:
    source = path.read_text(encoding="utf-8")
    lines = source.splitlines()
    symbols = _symbol_lines(lines)
    relpath = path.relative_to(root).as_posix()
    findings = []
    seen: set[tuple[int, str, str]] = set()

    for index, line in enumerate(lines):
        stripped = line.lstrip()
        if stripped.startswith(("//", "#", "*", "--")):
            # The audit prefers false negatives over treating pure comments as SQL.
            continue
        for match in OPERATION.finditer(line):
            operation = " ".join(match.group("operation").upper().split())
            relation = (match.group("object") or "UNKNOWN").strip()
            key = (index + 1, "sql", operation + ":" + relation)
            if key in seen:
                continue
            seen.add(key)
            findings.append(
                {
                    "path": relpath,
                    "line": index + 1,
                    "symbol_hint": symbols[index],
                    "kind": "sql_mutation_candidate",
                    "operation": operation,
                    "relation_hint": relation,
                    "category": _category(relpath, operation, relation),
                }
            )
        if path.suffix in (".rs", ".py"):
            for match in EXECUTION_SINK.finditer(line):
                sink = " ".join(match.group("sink").split())
                key = (index + 1, "sink", sink)
                if key in seen:
                    continue
                seen.add(key)
                findings.append(
                    {
                        "path": relpath,
                        "line": index + 1,
                        "symbol_hint": symbols[index],
                        "kind": "sql_execution_sink",
                        "operation": sink,
                        "relation_hint": "DYNAMIC_OR_UNKNOWN",
                        "category": "manual_review",
                    }
                )

    # Hints can help direct manual review, but never certify authorization.
    by_symbol: dict[str, set[str]] = {}
    current = "<module>"
    for line in lines:
        matched = FUNCTION.match(line)
        if matched:
            current = matched.group(1)
        by_symbol.setdefault(current, set()).update(
            marker for marker in GUARD_HINTS if marker in line
        )
    for finding in findings:
        finding["guard_hints_not_proof"] = sorted(
            by_symbol.get(str(finding["symbol_hint"]), set())
        )
    return findings


def collect(root: Path) -> dict[str, object]:
    root = root.resolve()
    paths = discover(root)
    findings = []
    for path in paths:
        findings.extend(inventory_file(path, root))
    findings.sort(key=lambda item: (item["path"], item["line"], item["kind"], item["operation"]))
    summary = dict(sorted(Counter(str(item["category"]) for item in findings).items()))
    kinds = dict(sorted(Counter(str(item["kind"]) for item in findings).items()))
    return {
        "schema_version": 1,
        "scope": "C10-B2 static candidates, not a complete SQL or fencing proof",
        "files_scanned": len(paths),
        "findings_count": len(findings),
        "by_category": summary,
        "by_kind": kinds,
        "findings": findings,
    }


def markdown(report: dict[str, object]) -> str:
    lines = [
        "# C10-B2 SQL mutation source inventory",
        "",
        "> Static candidates only. Dynamically composed SQL, authorization,",
        "> transaction boundaries and actual reachable code need manual review.",
        "",
        f"Files scanned: {report['files_scanned']}; findings: {report['findings_count']}.",
        "",
        "| Path:line | Function hint | Kind | Operation | Object hint | Category |",
        "|---|---|---|---|---|---|",
    ]
    for row in report["findings"]:
        fields = (
            f"\`{row['path']}:{row['line']}\`",
            f"\`{row['symbol_hint']}\`",
            str(row["kind"]),
            f"\`{row['operation']}\`",
            f"\`{row['relation_hint']}\`",
            str(row["category"]),
        )
        lines.append("| " + " | ".join(value.replace("|", "\\|") for value in fields) + " |")
    return "\n".join(lines) + "\n"


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--root", type=Path, default=Path(__file__).resolve().parents[2],
        help="FOD repository checkout to scan",
    )
    parser.add_argument("--format", choices=("json", "markdown"), default="json")
    parser.add_argument("--fail-on-empty", action="store_true")
    args = parser.parse_args()
    report = collect(args.root)
    if args.format == "markdown":
        print(markdown(report), end="")
    else:
        print(json.dumps(report, ensure_ascii=False, indent=2, sort_keys=True))
    return int(args.fail_on_empty and not report["findings_count"])


if __name__ == "__main__":
    raise SystemExit(main())
