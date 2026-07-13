#!/usr/bin/env bash
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1

set -euo pipefail

MOUNTPOINT_PATH="${1:-.}"
SCHEMA_NAME="${FOD_SCHEMA_NAME:-fod}"

if [[ ! -d "${MOUNTPOINT_PATH}" ]]; then
    printf 'Mountpoint does not exist or is not a directory: %s\n' "${MOUNTPOINT_PATH}" >&2
    exit 2
fi

if [[ ! "${SCHEMA_NAME}" =~ ^[A-Za-z_][A-Za-z0-9_]*$ ]]; then
    printf 'Invalid FOD_SCHEMA_NAME: %s\n' "${SCHEMA_NAME}" >&2
    exit 2
fi

MOUNTPOINT_PATH="$(cd "${MOUNTPOINT_PATH}" && pwd -P)"

printf 'FOD space accounting report\n'
printf 'generated_at=%s\n' "$(date --iso-8601=seconds)"
printf 'mountpoint=%s\n' "${MOUNTPOINT_PATH}"
printf '\n[findmnt]\n'
findmnt -T "${MOUNTPOINT_PATH}" -o TARGET,SOURCE,FSTYPE,OPTIONS || true

printf '\n[df bytes]\n'
df -B1 "${MOUNTPOINT_PATH}"
printf '\n[df inodes]\n'
df -i "${MOUNTPOINT_PATH}"

printf '\n[du totals]\n'
printf 'allocated_bytes\t'
du -x -B1 -s "${MOUNTPOINT_PATH}" | awk '{print $1}'
printf 'apparent_bytes\t'
du -x --apparent-size -B1 -s "${MOUNTPOINT_PATH}" | awk '{print $1}'

printf '\n[file stat: path logical_bytes st_blocks_512 allocated_bytes]\n'
find "${MOUNTPOINT_PATH}" -xdev -type f -printf '%p\t%s\t%b\n' \
    | LC_ALL=C sort \
    | awk -F '\t' '{printf "%s\t%s\t%s\t%s\n", $1, $2, $3, $3 * 512}'

printf '\n[file totals]\n'
find "${MOUNTPOINT_PATH}" -xdev -type f -printf '%s\t%b\n' \
    | awk -F '\t' '
        { logical += $1; blocks += $2 }
        END {
            printf "logical_bytes\t%.0f\n", logical
            printf "st_blocks_512\t%.0f\n", blocks
            printf "allocated_bytes\t%.0f\n", blocks * 512
        }
    '

if command -v psql >/dev/null 2>&1 && [[ -n "${PGDATABASE:-}" ]]; then
    printf '\n[PostgreSQL FOD payload]\n'
    psql -X -v ON_ERROR_STOP=1 -At -F $'\t' <<SQL
SELECT 'logical_file_bytes', COALESCE(SUM(size), 0)::bigint
FROM ${SCHEMA_NAME}.files;
SELECT 'unique_block_payload_bytes', COALESCE(SUM(octet_length(data)), 0)::bigint
FROM ${SCHEMA_NAME}.data_blocks;
SELECT 'unique_extent_payload_bytes', COALESCE(SUM(octet_length(payload)), 0)::bigint
FROM ${SCHEMA_NAME}.data_extents;
SELECT 'unique_payload_bytes',
       (SELECT COALESCE(SUM(octet_length(data)), 0) FROM ${SCHEMA_NAME}.data_blocks)
       +
       (SELECT COALESCE(SUM(octet_length(payload)), 0) FROM ${SCHEMA_NAME}.data_extents);
SELECT 'file_rows', COUNT(*)::bigint FROM ${SCHEMA_NAME}.files;
SELECT 'distinct_data_objects', COUNT(DISTINCT data_object_id)::bigint
FROM ${SCHEMA_NAME}.files
WHERE data_object_id IS NOT NULL;
SQL
else
    printf '\n[PostgreSQL FOD payload]\n'
    printf 'skipped: set PGDATABASE (and, when needed, PGHOST/PGPORT/PGUSER/PGPASSWORD) to include database totals\n'
fi
