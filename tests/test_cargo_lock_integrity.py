#!/usr/bin/env python3
from __future__ import annotations

import re
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
LOCK = ROOT / "Cargo.lock"
VERSION_FILE = ROOT / "fod_version.txt"

checksum_re = re.compile(rb'^checksum = "([0-9a-f]{64})"$')
package_name_re = re.compile(r'^name = "([^"]+)"$')
package_version_re = re.compile(r'^version = "([^"]+)"$')

raw_lines = LOCK.read_bytes().splitlines()
checksum_count = 0
for lineno, raw in enumerate(raw_lines, 1):
    if raw.startswith(b"checksum = "):
        checksum_count += 1
        if checksum_re.fullmatch(raw) is None:
            raise SystemExit(
                f"Cargo.lock invalid registry checksum at line {lineno}: {raw!r}"
            )

if checksum_count == 0:
    raise SystemExit("Cargo.lock contains no registry checksums")

release_version = VERSION_FILE.read_text(encoding="utf-8").strip()
expected_packages = {
    "fod-lib",
    "fod-rust-fuse",
    "fod-rust-hotpath",
    "fod-rust-indexer",
    "fod-rust-mkfs",
    "fod-rust-monitor",
    "fod-rust-runtime",
}
seen: dict[str, str] = {}
current_name: str | None = None
for line in LOCK.read_text(encoding="utf-8").splitlines():
    name_match = package_name_re.fullmatch(line)
    if name_match:
        current_name = name_match.group(1)
        continue
    version_match = package_version_re.fullmatch(line)
    if version_match and current_name in expected_packages:
        seen[current_name] = version_match.group(1)
        current_name = None

missing = expected_packages - seen.keys()
if missing:
    raise SystemExit(f"Cargo.lock missing FOD packages: {sorted(missing)}")

wrong = {name: version for name, version in seen.items() if version != release_version}
if wrong:
    raise SystemExit(
        f"Cargo.lock FOD package versions do not match {release_version}: {wrong}"
    )

print(
    f"OK cargo-lock-integrity version={release_version} "
    f"registry_checksums={checksum_count} fod_packages={len(seen)}"
)
