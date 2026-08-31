#!/usr/bin/env bash
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1
#
# Low-impact host storage diagnostic for FOD performance work. This script does
# not run the 1 GiB FOD workload and does not prune Docker data. It records the
# storage topology used by the repository and Docker, mount/filesystem details,
# available NVMe/SMART health information, temperatures, diskstats and a small
# 4 KiB fsync latency series.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "${ROOT}"

RUN_ID="$(date -u +%Y%m%dT%H%M%SZ)"
HEAD_SHORT="$(git rev-parse --short HEAD 2>/dev/null || echo unknown)"
HOST_NAME="$(hostname -s 2>/dev/null || hostname)"
OUT="${FOD_STORAGE_DIAG_OUT:-${ROOT}/artifacts/perf/${HEAD_SHORT}/${HOST_NAME}-storage-diagnostic-${RUN_ID}}"
FSYNC_COUNT="${FOD_STORAGE_DIAG_FSYNC_COUNT:-30}"
FSYNC_INTERVAL_SECONDS="${FOD_STORAGE_DIAG_FSYNC_INTERVAL_SECONDS:-2}"

case "${FSYNC_COUNT}" in ''|*[!0-9]*) echo "FOD_STORAGE_DIAG_FSYNC_COUNT must be a positive integer" >&2; exit 2;; esac
case "${FSYNC_INTERVAL_SECONDS}" in ''|*[!0-9]*) echo "FOD_STORAGE_DIAG_FSYNC_INTERVAL_SECONDS must be a non-negative integer" >&2; exit 2;; esac
(( FSYNC_COUNT >= 1 )) || { echo "FOD_STORAGE_DIAG_FSYNC_COUNT must be >= 1" >&2; exit 2; }

for cmd in awk date df findmnt git hostname lsblk python3 sync; do
    command -v "${cmd}" >/dev/null 2>&1 || { echo "Missing required command: ${cmd}" >&2; exit 2; }
done

mkdir -p "${OUT}"
DOCKER_ROOT=""
if command -v docker >/dev/null 2>&1; then
    DOCKER_ROOT="$(docker info --format '{{.DockerRootDir}}' 2>/dev/null || true)"
fi

printf 'captured_at=%s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" >"${OUT}/meta.txt"
printf 'host=%s\nhead=%s\nrepo=%s\ndocker_root=%s\n' "${HOST_NAME}" "${HEAD_SHORT}" "${ROOT}" "${DOCKER_ROOT}" >>"${OUT}/meta.txt"
uname -a >"${OUT}/uname.txt" 2>&1 || true

{
    echo '=== lsblk ==='
    lsblk -e 7 -o NAME,KNAME,PKNAME,TYPE,SIZE,ROTA,TRAN,MODEL,FSTYPE,FSVER,FSAVAIL,FSUSE%,MOUNTPOINTS
    echo
    echo '=== target mounts ==='
    for target in "${ROOT}" "${OUT}" ${DOCKER_ROOT:+"${DOCKER_ROOT}"}; do
        echo "--- ${target} ---"
        findmnt -T "${target}" -o TARGET,SOURCE,FSTYPE,OPTIONS,FSROOT -n || true
        df -hT "${target}" || true
    done
} >"${OUT}/storage-topology.txt" 2>&1

{
    echo '=== meminfo writeback ==='
    grep -E '^(MemTotal|MemAvailable|Dirty|Writeback|WritebackTmp|Cached|Buffers):' /proc/meminfo || true
    echo
    echo '=== pressure io ==='
    cat /proc/pressure/io 2>/dev/null || true
    echo
    echo '=== vm dirty settings ==='
    for key in dirty_background_bytes dirty_background_ratio dirty_bytes dirty_expire_centisecs dirty_ratio dirty_writeback_centisecs; do
        printf '%s=' "$key"
        cat "/proc/sys/vm/${key}" 2>/dev/null || echo unavailable
    done
} >"${OUT}/host-writeback.txt" 2>&1

cat /proc/diskstats >"${OUT}/diskstats-before.txt" 2>/dev/null || true

if command -v iostat >/dev/null 2>&1; then
    iostat -xz 1 10 >"${OUT}/iostat.txt" 2>&1 || true
else
    echo 'iostat unavailable' >"${OUT}/iostat.txt"
fi

{
    echo '=== NVMe sysfs ==='
    for controller in /sys/class/nvme/nvme*; do
        [[ -e "${controller}" ]] || continue
        name="$(basename "${controller}")"
        echo "--- ${name} ---"
        for field in model serial firmware_rev state; do
            if [[ -r "${controller}/${field}" ]]; then
                printf '%s=' "$field"
                cat "${controller}/${field}"
            fi
        done
        for hwmon in "${controller}"/device/hwmon/hwmon*; do
            [[ -d "${hwmon}" ]] || continue
            for temp in "${hwmon}"/temp*_input; do
                [[ -r "${temp}" ]] || continue
                raw="$(cat "${temp}" 2>/dev/null || echo 0)"
                label_file="${temp%_input}_label"
                label="$(basename "${temp%_input}")"
                [[ -r "${label_file}" ]] && label="$(cat "${label_file}")"
                awk -v label="${label}" -v raw="${raw}" 'BEGIN {printf "%s=%.1f C\n", label, raw/1000.0}'
            done
        done
    done
} >"${OUT}/nvme-sysfs.txt" 2>&1

if command -v nvme >/dev/null 2>&1; then
    : >"${OUT}/nvme-smart.txt"
    for dev in /dev/nvme[0-9]; do
        [[ -e "${dev}" ]] || continue
        echo "=== ${dev} ===" >>"${OUT}/nvme-smart.txt"
        nvme smart-log "${dev}" >>"${OUT}/nvme-smart.txt" 2>&1 || true
    done
else
    echo 'nvme command unavailable' >"${OUT}/nvme-smart.txt"
fi

if command -v smartctl >/dev/null 2>&1; then
    : >"${OUT}/smartctl.txt"
    while read -r dev _; do
        [[ -b "/dev/${dev}" ]] || continue
        echo "=== /dev/${dev} ===" >>"${OUT}/smartctl.txt"
        smartctl -a "/dev/${dev}" >>"${OUT}/smartctl.txt" 2>&1 || true
    done < <(lsblk -dn -o KNAME,TYPE | awk '$2=="disk" {print $1, $2}')
else
    echo 'smartctl command unavailable' >"${OUT}/smartctl.txt"
fi

sync
python3 - "${OUT}/.fsync-probe" "${FSYNC_COUNT}" "${FSYNC_INTERVAL_SECONDS}" >"${OUT}/fsync.tsv" <<'PY'
import os
import statistics
import sys
import time

path = sys.argv[1]
count = int(sys.argv[2])
interval = int(sys.argv[3])
values = []
print("sample\ttimestamp\tfsync_ms")
for index in range(1, count + 1):
    started = time.perf_counter_ns()
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
    try:
        os.write(fd, b"x" * 4096)
        os.fsync(fd)
    finally:
        os.close(fd)
    elapsed = (time.perf_counter_ns() - started) / 1_000_000.0
    values.append(elapsed)
    print(f"{index}\t{time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime())}\t{elapsed:.3f}", flush=True)
    if index != count and interval:
        time.sleep(interval)
try:
    os.unlink(path)
except FileNotFoundError:
    pass
s = sorted(values)
p95 = s[min(len(s)-1, max(0, int((len(s)-1)*0.95)))]
print(f"summary\tmedian_ms={statistics.median(values):.3f}\tp95_ms={p95:.3f}\tmax_ms={max(values):.3f}")
PY

cat /proc/diskstats >"${OUT}/diskstats-after.txt" 2>/dev/null || true

{
    echo '=== final writeback ==='
    grep -E '^(Dirty|Writeback|WritebackTmp):' /proc/meminfo || true
    echo '=== final io pressure ==='
    cat /proc/pressure/io 2>/dev/null || true
} >"${OUT}/host-after.txt" 2>&1

summary_line="$(tail -n 1 "${OUT}/fsync.tsv" 2>/dev/null || true)"
echo "storage_diagnostic_artifact_dir=${OUT}"
echo "fsync_${summary_line}"
echo 'OK: host storage stability diagnostic'
