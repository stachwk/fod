#!/usr/bin/env bash
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SCRIPT="${ROOT}/scripts/perf/diagnose_host_storage_stability.sh"

[[ -r "${SCRIPT}" ]] || { echo "Missing storage diagnostic: ${SCRIPT}" >&2; exit 1; }
bash -n "${SCRIPT}"

grep -F 'storage-topology.txt' "${SCRIPT}" >/dev/null
grep -F 'findmnt -T' "${SCRIPT}" >/dev/null
grep -F 'lsblk -e 7' "${SCRIPT}" >/dev/null
grep -F 'diskstats-before.txt' "${SCRIPT}" >/dev/null
grep -F 'diskstats-after.txt' "${SCRIPT}" >/dev/null
grep -F 'fsync.tsv' "${SCRIPT}" >/dev/null
grep -F 'nvme-smart.txt' "${SCRIPT}" >/dev/null
grep -F 'smartctl.txt' "${SCRIPT}" >/dev/null
grep -F '/proc/pressure/io' "${SCRIPT}" >/dev/null
grep -F 'FOD_STORAGE_DIAG_FSYNC_COUNT:-30' "${SCRIPT}" >/dev/null
grep -F 'FOD_STORAGE_DIAG_FSYNC_INTERVAL_SECONDS:-2' "${SCRIPT}" >/dev/null

if grep -Eq 'docker[[:space:]]+(system[[:space:]]+)?prune|docker[[:space:]]+volume[[:space:]]+prune|fstrim|blkdiscard|fio[[:space:]]' "${SCRIPT}"; then
    echo "Storage diagnostic must remain low-impact and non-destructive" >&2
    exit 1
fi

echo "OK: host storage stability diagnostic policy"
