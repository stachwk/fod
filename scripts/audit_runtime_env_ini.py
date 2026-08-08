#!/usr/bin/env python3
from __future__ import annotations
import argparse
import re
from pathlib import Path

ENV_LITERAL_RE = re.compile(r'["\'](FOD_[A-Z0-9_]+)["\']')
PRODUCTION_DIRS = (
    "rust_runtime/src", "rust_fuse/src", "rust_hotpath/src",
    "rust_monitor/src", "rust_mkfs/src", "rust_indexer/src",
)
SPECIAL_MAP = {
    "FOD_RUST_FUSE_READONLY": ("fod", "force_read_only"),
    "FOD_PG_HOST": ("database", "host"),
    "FOD_PG_PORT": ("database", "port"),
    "FOD_PG_DBNAME": ("database", "dbname"),
    "FOD_PG_USER": ("database", "user"),
    "FOD_PG_PASSWORD": ("database", "password"),
    "FOD_PG_SSLMODE": ("database", "sslmode"),
    "FOD_PG_SSLROOTCERT": ("database", "sslrootcert"),
    "FOD_PG_SSLCERT": ("database", "sslcert"),
    "FOD_PG_SSLKEY": ("database", "sslkey"),
    "FOD_PG_PRIMARY_HOSTS": ("database", "primary_hosts"),
    "FOD_PG_REPLICA_HOSTS": ("database", "replica_hosts"),
    "FOD_PG_HOSTS": ("database", "hosts"),
}
INTENTIONAL_ENV_ONLY = {
    "FOD_CONFIG": "config file selector",
    "FOD_DSN_CONNINFO": "internal bootstrap-to-FUSE PostgreSQL handoff",
    "FOD_SCHEMA_ADMIN_PASSWORD": "secret kept outside shareable INI",
    "FOD_PROFILE": "profile selector; [fod] profile is also supported",
    "FOD_DEBUG": "diagnostic bootstrap override",
    "FOD_DEBUG_SNAPSHOT": "diagnostic snapshot",
    "FOD_PROFILE_IO": "profiling instrumentation",
    "FOD_STRACE": "test/profiling switch",
    "FOD_STRACE_LABEL": "test/profiling metadata",
    "FOD_STRACE_SUMMARY_FILE": "test/profiling output",
    "FOD_TASK_OBSERVABILITY_INTERVAL_MS": "diagnostic sampling cadence",
    "FOD_POSTGRES_AUTOCOMMIT": "PostgreSQL requirement test override",
    "FOD_PROFILE_IO_VERBOSE": "verbose per-call I/O profiling; diagnostic only",
    "FOD_PG_OBSERVABILITY_INTERVAL_MS": "PostgreSQL lane observability sampler; default 5000 ms, valid 100..3600000 ms",
    "FOD_PG_POOL_LANES_ENABLED": "temporary opt-in for dedicated PostgreSQL read/write/control/lease pools",
    "FOD_PERSIST_COPY_SEND_BUFFER_BYTES": "experimental libpq COPY send-buffer tuning; default 1 MiB",
    "FOD_INDEXER_CONNINFO": "indexer-specific PostgreSQL conninfo override/handoff",
    "FOD_PROCESS_NAMES": "standalone monitor process-name selector",
    "FOD_REQUESTED_CAPABILITIES": "FUSE capability/compatibility diagnostic override",
    "FOD_RUNTIME_SCHEMA_DDL_LOCK_SQL": "internal/test schema DDL lock SQL override",
    "FOD_RUNTIME_SCHEMA_DDL_UNLOCK_SQL": "internal/test schema DDL unlock SQL override",
    "FOD_VERSION": "monitor/version diagnostic override",
    "FOD_VERSION_LABEL": "build-time version label; not runtime INI",
}
REQUIRED_ACTIVE = {
    ("fod", "fuse_event_threads"),
    ("fod", "fuse_clone_fd"),
    ("fod", "task_read_active_limit"),
    ("fod", "task_write_active_limit"),
}
REQUIRED_DOCUMENTED = REQUIRED_ACTIVE | {
    ("fod", "allow_other"),
    ("fod", "entry_timeout_seconds"),
    ("fod", "attr_timeout_seconds"),
    ("fod", "negative_timeout_seconds"),
}

def parse_keys(path: Path):
    active, documented = set(), set()
    section = ""
    section_re = re.compile(r"^\s*\[([^\]]+)\]\s*$")
    active_re = re.compile(r"^\s*([A-Za-z0-9_.-]+)\s*=")
    comment_re = re.compile(r"^\s*[#;]\s*([A-Za-z0-9_.-]+)\s*=")
    for raw in path.read_text(encoding="utf-8").splitlines():
        m = section_re.match(raw)
        if m:
            section = m.group(1).strip().lower()
            continue
        m = active_re.match(raw)
        if m and section:
            item = (section, m.group(1).lower())
            active.add(item); documented.add(item)
            continue
        m = comment_re.match(raw)
        if m and section:
            documented.add((section, m.group(1).lower()))
    return active, documented

def expected_key(env_name: str):
    return SPECIAL_MAP.get(env_name, ("fod", env_name.removeprefix("FOD_").lower()))

def scan_envs(root: Path):
    result = {}
    for rel in PRODUCTION_DIRS:
        base = root / rel
        if not base.exists():
            continue
        for path in base.rglob("*.rs"):
            if "/tests/" in path.as_posix():
                continue
            text = path.read_text(encoding="utf-8", errors="replace")
            for env_name in ENV_LITERAL_RE.findall(text):
                result.setdefault(env_name, set()).add(path.relative_to(root).as_posix())
    return result

def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", default=".")
    parser.add_argument("--output")
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    root = Path(args.root).resolve()
    ini_paths = [root / "fod_config.ini", root / "fod_config.example.ini"]
    active = {}
    documented = {}
    for path in ini_paths:
        active[path.name], documented[path.name] = parse_keys(path)

    missing_active = [
        (name, item)
        for item in sorted(REQUIRED_ACTIVE)
        for name, keys in active.items()
        if item not in keys
    ]
    missing_documented = [
        (name, item)
        for item in sorted(REQUIRED_DOCUMENTED)
        for name, keys in documented.items()
        if item not in keys
    ]

    rows = []
    for env_name, sources in sorted(scan_envs(root).items()):
        item = expected_key(env_name)
        active_files = [name for name, keys in active.items() if item in keys]
        documented_files = [name for name, keys in documented.items() if item in keys]
        if env_name in INTENTIONAL_ENV_ONLY:
            classification = "intentional-env-only"
        elif len(active_files) == 2:
            classification = "active-in-both-ini"
        elif len(documented_files) == 2:
            classification = "documented-in-both-ini"
        else:
            classification = "env-only-or-incomplete-ini"
        note = INTENTIONAL_ENV_ONLY.get(env_name, "")
        if classification == "env-only-or-incomplete-ini" and not note:
            note = "review required before promoting this runtime literal to persistent configuration"
        rows.append((env_name, item, classification, active_files, documented_files, sources, note))

    lines = [
        "# Runtime environment vs INI audit", "",
        "Generated from quoted `FOD_*` literals in production Rust sources. Rust identifiers with similar names are excluded.",
        "",
        "| environment variable | expected INI key | classification | active in | documented in | source | note |",
        "| --- | --- | --- | --- | --- | --- | --- |",
    ]
    for env_name, (section, key), classification, active_files, documented_files, sources, note in rows:
        lines.append(
            f"| `{env_name}` | `[{section}] {key}` | {classification} | "
            f"{', '.join(active_files) or '-'} | {', '.join(documented_files) or '-'} | "
            f"{', '.join(sorted(sources))} | {note or '-'} |"
        )
    lines += [
        "", "## Classification", "",
        "- `active-in-both-ini`: active assignment exists in both base INI files.",
        "- `documented-in-both-ini`: represented in both files but may stay commented to preserve defaults.",
        "- `intentional-env-only`: secret, selector, internal handoff, or diagnostic/test control.",
        "- `env-only-or-incomplete-ini`: needs an explicit design decision.",
        "",
    ]
    if missing_active:
        lines += ["## Missing required active settings", ""]
        for name, (section, key) in missing_active:
            lines.append(f"- `{name}`: `[{section}] {key}`")
        lines.append("")
    if missing_documented:
        lines += ["## Missing required documented settings", ""]
        for name, (section, key) in missing_documented:
            lines.append(f"- `{name}`: `[{section}] {key}`")
        lines.append("")

    output = "\n".join(lines).rstrip() + "\n"
    if args.output:
        Path(args.output).write_text(output, encoding="utf-8")
    print(output, end="")
    return 1 if args.check and (missing_active or missing_documented) else 0

if __name__ == "__main__":
    raise SystemExit(main())
