#!/usr/bin/env bash
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
source "${ROOT}/tests/integration/fod_testlib.sh"
fod_test_setup "${ROOT}"
fod_test_make_mountpoint /tmp/fod-df
OUTPUT_DIR="$(mktemp -d /tmp/fod-df-output.XXXXXX)"
export FOD_PG_VISIBLE_PATH="${OUTPUT_DIR}"
export FOD_STATFS_CACHE_TTL_SECONDS=0
ORIGINAL_MAX_FS_SIZE_BYTES=""
read -r host_fragment_size host_available_blocks < <(
  stat -f -c '%S %a' "${FOD_PG_VISIBLE_PATH}"
)
host_available_bytes=$((host_fragment_size * host_available_blocks))

cleanup() {
  fod_test_cleanup
  if [[ -n "${ORIGINAL_MAX_FS_SIZE_BYTES}" ]]; then
    ORIGINAL_MAX_FS_SIZE_BYTES="${ORIGINAL_MAX_FS_SIZE_BYTES}" \
      "${VENV_PYTHON}" - <<'PY' || true
import os
import psycopg2

connection = psycopg2.connect(
    host=os.environ.get("POSTGRES_HOST", "127.0.0.1"),
    port=os.environ.get("POSTGRES_PORT", "5432"),
    dbname=os.environ.get("POSTGRES_DB", "foddbname"),
    user=os.environ.get("POSTGRES_USER", "foduser"),
    password=os.environ.get("POSTGRES_PASSWORD", "cichosza"),
)
connection.autocommit = True
with connection.cursor() as cursor:
    cursor.execute(
        "UPDATE fod.config SET value = %s WHERE key = 'max_fs_size_bytes'",
        (int(os.environ["ORIGINAL_MAX_FS_SIZE_BYTES"]),),
    )
connection.close()
PY
  fi
  rm -rf "${OUTPUT_DIR}"
}

trap cleanup EXIT

fod_test_init_schema
read -r ORIGINAL_MAX_FS_SIZE_BYTES test_max_fs_size_bytes < <(
  "${VENV_PYTHON}" - <<'PY'
import os
import psycopg2

connection = psycopg2.connect(
    host=os.environ.get("POSTGRES_HOST", "127.0.0.1"),
    port=os.environ.get("POSTGRES_PORT", "5432"),
    dbname=os.environ.get("POSTGRES_DB", "foddbname"),
    user=os.environ.get("POSTGRES_USER", "foduser"),
    password=os.environ.get("POSTGRES_PASSWORD", "cichosza"),
)
connection.autocommit = True
with connection.cursor() as cursor:
    cursor.execute(
        """
        SELECT
            (SELECT value FROM fod.config WHERE key = 'max_fs_size_bytes'),
            (SELECT COUNT(*)::bigint FROM fod.data_blocks)
                * (SELECT value FROM fod.config WHERE key = 'block_size')
                + COALESCE(
                    (SELECT SUM(used_bytes)::bigint FROM fod.data_extents),
                    0
                )
        """
    )
    original_limit, used_bytes = cursor.fetchone()
    test_limit = int(used_bytes) + 128 * 1024 * 1024
    cursor.execute(
        "UPDATE fod.config SET value = %s WHERE key = 'max_fs_size_bytes'",
        (test_limit,),
    )
print(int(original_limit), test_limit)
connection.close()
PY
)
fod_test_start_mount "${MOUNTPOINT}"
rm -f \
  "${MOUNTPOINT}/accounting-link" \
  "${MOUNTPOINT}/accounting-small.bin" \
  "${MOUNTPOINT}/accounting-large.bin" \
  "${MOUNTPOINT}/accounting-sparse.bin" \
  "${MOUNTPOINT}/accounting-shared-source.bin" \
  "${MOUNTPOINT}/accounting-shared-copy.bin"

python3 - "${MOUNTPOINT}" <<'PY'
import os
from pathlib import Path
import sys

mountpoint = Path(sys.argv[1])
(mountpoint / "accounting-small.bin").write_bytes(b"a" * 4097)
(mountpoint / "accounting-large.bin").write_bytes(b"b" * 1048577)
with (mountpoint / "accounting-sparse.bin").open("wb") as sparse:
    sparse.seek(1048576)
    sparse.write(b"c")
shared_source = mountpoint / "accounting-shared-source.bin"
shared_copy = mountpoint / "accounting-shared-copy.bin"
shared_source.write_bytes(b"s" * 65536)
shared_copy.write_bytes(b"")
with shared_source.open("rb") as source, shared_copy.open("r+b") as destination:
    copied = os.copy_file_range(
        source.fileno(),
        destination.fileno(),
        shared_source.stat().st_size,
        offset_src=0,
        offset_dst=0,
    )
if copied != shared_source.stat().st_size:
    raise AssertionError(f"short shared-object copy: {copied}")
PY
sync

for file in \
  "${MOUNTPOINT}/accounting-small.bin" \
  "${MOUNTPOINT}/accounting-large.bin" \
  "${MOUNTPOINT}/accounting-shared-source.bin" \
  "${MOUNTPOINT}/accounting-shared-copy.bin"
do
  read -r logical_size stat_blocks stat_block_unit io_block_size < <(
    stat -c '%s %b %B %o' "${file}"
  )
  expected_blocks=$((((logical_size + io_block_size - 1) / io_block_size) * io_block_size / 512))

  if [[ "${stat_block_unit}" -ne 512 ]]; then
    printf 'unexpected stat block unit for %s: got=%s expected=512\n' \
      "${file}" "${stat_block_unit}" >&2
    exit 1
  fi

  if [[ "${stat_blocks}" -ne "${expected_blocks}" ]]; then
    printf 'unexpected st_blocks for %s: size=%s got=%s expected=%s\n' \
      "${file}" "${logical_size}" "${stat_blocks}" "${expected_blocks}" >&2
    exit 1
  fi

  allocated_bytes="$(du -B1 "${file}" | awk '{print $1}')"
  apparent_bytes="$(du --apparent-size -B1 "${file}" | awk '{print $1}')"
  expected_allocated_bytes=$((expected_blocks * 512))

  if [[ "${allocated_bytes}" -ne "${expected_allocated_bytes}" ]]; then
    printf 'unexpected du allocation for %s: got=%s expected=%s\n' \
      "${file}" "${allocated_bytes}" "${expected_allocated_bytes}" >&2
    exit 1
  fi

  if [[ "${apparent_bytes}" -ne "${logical_size}" ]]; then
    printf 'unexpected apparent size for %s: got=%s expected=%s\n' \
      "${file}" "${apparent_bytes}" "${logical_size}" >&2
    exit 1
  fi
done

sparse_file="${MOUNTPOINT}/accounting-sparse.bin"
read -r sparse_size sparse_blocks < <(stat -c '%s %b' "${sparse_file}")
sparse_logical_blocks=$(((sparse_size + 511) / 512))
if [[ "${sparse_blocks}" -le 0 || "${sparse_blocks}" -ge "${sparse_logical_blocks}" ]]; then
  printf 'unexpected sparse allocation: size=%s blocks=%s logical_blocks=%s\n' \
    "${sparse_size}" "${sparse_blocks}" "${sparse_logical_blocks}" >&2
  exit 1
fi

capture_accounting_contract() {
  local label="$1"
  read -r persisted_bytes reserved_bytes shared_source_object shared_copy_object shared_references logical_bytes < <(
    "${VENV_PYTHON}" - <<'PY'
import os
import psycopg2

connection = psycopg2.connect(
    host=os.environ.get("POSTGRES_HOST", "127.0.0.1"),
    port=os.environ.get("POSTGRES_PORT", "5432"),
    dbname=os.environ.get("POSTGRES_DB", "foddbname"),
    user=os.environ.get("POSTGRES_USER", "foduser"),
    password=os.environ.get("POSTGRES_PASSWORD", "cichosza"),
)
with connection.cursor() as cursor:
    cursor.execute(
        """
        SELECT
            (SELECT COUNT(*)::bigint FROM fod.data_blocks)
                * (SELECT value FROM fod.config WHERE key = 'block_size')
                + COALESCE(
                    (SELECT SUM(used_bytes)::bigint FROM fod.data_extents),
                    0
                ),
            COALESCE(
                (SELECT SUM(reserved_bytes)::bigint
                 FROM fod.payload_capacity_reservations
                 WHERE expires_at > NOW()),
                0
            ),
            source.data_object_id,
            copy.data_object_id,
            object.reference_count,
            (SELECT COALESCE(SUM(size)::bigint, 0) FROM fod.files)
        FROM fod.files source
        JOIN fod.files copy ON copy.name = 'accounting-shared-copy.bin'
                            AND copy.id_directory IS NULL
        JOIN fod.data_objects object
          ON object.id_data_object = source.data_object_id
        WHERE source.name = 'accounting-shared-source.bin'
          AND source.id_directory IS NULL
        """
    )
    row = cursor.fetchone()
    if row is None:
        raise AssertionError("missing shared accounting files")
    print(*(int(value) for value in row))
connection.close()
PY
  )

  if [[ "${shared_source_object}" -ne "${shared_copy_object}" ]]; then
    printf 'shared copy does not reuse data object: source=%s copy=%s\n' \
      "${shared_source_object}" "${shared_copy_object}" >&2
    exit 1
  fi
  if [[ "${shared_references}" -ne 2 ]]; then
    printf 'unexpected shared data object reference count: got=%s expected=2\n' \
      "${shared_references}" >&2
    exit 1
  fi

  read -r contract_bsize contract_blocks contract_free < <(
    stat -f -c '%S %b %f' "${MOUNTPOINT}"
  )
  accounted_bytes=$((persisted_bytes + reserved_bytes))
  expected_used_blocks=$(((accounted_bytes + contract_bsize - 1) / contract_bsize))
  actual_used_blocks=$((contract_blocks - contract_free))
  if [[ "${actual_used_blocks}" -ne "${expected_used_blocks}" ]]; then
    printf 'statfs/PostgreSQL accounting mismatch (%s): got_blocks=%s expected_blocks=%s persisted=%s reserved=%s\n' \
      "${label}" "${actual_used_blocks}" "${expected_used_blocks}" \
      "${persisted_bytes}" "${reserved_bytes}" >&2
    exit 1
  fi

  shared_source_du="$(du -B1 "${MOUNTPOINT}/accounting-shared-source.bin" | awk '{print $1}')"
  shared_copy_du="$(du -B1 "${MOUNTPOINT}/accounting-shared-copy.bin" | awk '{print $1}')"
  if [[ "${shared_source_du}" -ne "${shared_copy_du}" || "${shared_source_du}" -le 0 ]]; then
    printf 'shared files have inconsistent attributed allocation: source=%s copy=%s\n' \
      "${shared_source_du}" "${shared_copy_du}" >&2
    exit 1
  fi

  ACCOUNTING_PERSISTED_BYTES="${persisted_bytes}"
  ACCOUNTING_RESERVED_BYTES="${reserved_bytes}"
  ACCOUNTING_SHARED_OBJECT="${shared_source_object}"
  ACCOUNTING_LOGICAL_BYTES="${logical_bytes}"
  printf 'OK accounting contract (%s) persisted=%s reserved=%s logical=%s shared_object=%s attributed_du_each=%s\n' \
    "${label}" "${persisted_bytes}" "${reserved_bytes}" "${logical_bytes}" \
    "${shared_source_object}" "${shared_source_du}"
}

capture_accounting_contract before-remount
persisted_before_remount="${ACCOUNTING_PERSISTED_BYTES}"
reserved_before_remount="${ACCOUNTING_RESERVED_BYTES}"
shared_object_before_remount="${ACCOUNTING_SHARED_OBJECT}"
logical_before_remount="${ACCOUNTING_LOGICAL_BYTES}"

read -r fs_bsize fs_frsize fs_blocks fs_free_blocks fs_inodes fs_free_inodes fs_name_max < <(
  stat -f -c '%s %S %b %f %c %d %l' "${MOUNTPOINT}"
)

if [[ "${fs_bsize}" -ne "${fs_frsize}" ]]; then
  printf 'unexpected statfs block sizes: bsize=%s frsize=%s\n' \
    "${fs_bsize}" "${fs_frsize}" >&2
  exit 1
fi

reported_available_bytes=$((fs_free_blocks * fs_frsize))
if [[ "${reported_available_bytes}" -gt "${host_available_bytes}" ]]; then
  printf 'statfs exceeds host available bytes: reported=%s host_before_mount=%s\n' \
    "${reported_available_bytes}" "${host_available_bytes}" >&2
  exit 1
fi

if [[ "${fs_name_max}" -ne 255 ]]; then
  printf 'unexpected statfs name length: got=%s expected=255\n' \
    "${fs_name_max}" >&2
  exit 1
fi

if [[ "${fs_free_inodes}" -le 0 || "${fs_inodes}" -le "${fs_free_inodes}" ]]; then
  printf 'unexpected statfs inode capacity: inodes=%s ifree=%s\n' \
    "${fs_inodes}" "${fs_free_inodes}" >&2
  exit 1
fi

ln -s accounting-small.bin "${MOUNTPOINT}/accounting-link"
read -r symlink_fs_inodes symlink_fs_free_inodes < <(
  stat -f -c '%c %d' "${MOUNTPOINT}"
)
if [[ "${symlink_fs_inodes}" -ne $((fs_inodes + 1)) ]]; then
  printf 'symlink did not increment statfs inode count: before=%s after=%s\n' \
    "${fs_inodes}" "${symlink_fs_inodes}" >&2
  exit 1
fi
if [[ "${symlink_fs_free_inodes}" -ne "${fs_free_inodes}" ]]; then
  printf 'symlink unexpectedly changed virtual inode headroom: before=%s after=%s\n' \
    "${fs_free_inodes}" "${symlink_fs_free_inodes}" >&2
  exit 1
fi

DF_BYTES_OUTPUT="${OUTPUT_DIR}/df.ph"
DF_INODES_OUTPUT="${OUTPUT_DIR}/df.phi"
df -Ph "${MOUNTPOINT}" > "${DF_BYTES_OUTPUT}"
df -Phi "${MOUNTPOINT}" > "${DF_INODES_OUTPUT}"

awk -v mount="${MOUNTPOINT}" '
  NR == 2 {
    if ($6 != mount) {
      print "unexpected mountpoint in df -Ph: " $6
      exit 1
    }
    if ($2 == "" || $3 == "" || $4 == "" || $5 == "") {
      print "missing df -Ph fields"
      exit 1
    }
  }
' "${DF_BYTES_OUTPUT}"

awk -v mount="${MOUNTPOINT}" '
  NR == 2 {
    if ($6 != mount) {
      print "unexpected mountpoint in df -Phi: " $6
      exit 1
    }
    if ($2 == "" || $3 == "" || $4 == "" || $5 == "") {
      print "missing df -Phi fields"
      exit 1
    }
    if ($5 == "100%") {
      print "df -Phi falsely reports inode exhaustion"
      exit 1
    }
  }
' "${DF_INODES_OUTPUT}"

if command -v fusermount3 >/dev/null 2>&1; then
  fusermount3 -u "${MOUNTPOINT}"
elif command -v fusermount >/dev/null 2>&1; then
  fusermount -u "${MOUNTPOINT}"
else
  umount "${MOUNTPOINT}"
fi
if [[ -n "${FOD_PID}" ]] && kill -0 "${FOD_PID}" >/dev/null 2>&1; then
  kill "${FOD_PID}" >/dev/null 2>&1 || true
  wait "${FOD_PID}" >/dev/null 2>&1 || true
fi
FOD_PID=""
fod_test_start_mount "${MOUNTPOINT}"

python3 - "${MOUNTPOINT}" <<'PY'
from pathlib import Path
import sys

mountpoint = Path(sys.argv[1])
source = (mountpoint / "accounting-shared-source.bin").read_bytes()
copy = (mountpoint / "accounting-shared-copy.bin").read_bytes()
if source != b"s" * 65536 or copy != source:
    raise AssertionError("shared payload changed across remount")
sparse = mountpoint / "accounting-sparse.bin"
if sparse.stat().st_size != 1048577:
    raise AssertionError(f"sparse size changed across remount: {sparse.stat().st_size}")
with sparse.open("rb") as stream:
    stream.seek(1048576)
    if stream.read(1) != b"c":
        raise AssertionError("sparse tail changed across remount")
PY

capture_accounting_contract after-remount
if [[ "${ACCOUNTING_PERSISTED_BYTES}" -ne "${persisted_before_remount}" ||
      "${ACCOUNTING_RESERVED_BYTES}" -ne "${reserved_before_remount}" ||
      "${ACCOUNTING_SHARED_OBJECT}" -ne "${shared_object_before_remount}" ||
      "${ACCOUNTING_LOGICAL_BYTES}" -ne "${logical_before_remount}" ]]; then
  echo "space-accounting contract changed across remount" >&2
  exit 1
fi

rm -f \
  "${MOUNTPOINT}/accounting-link" \
  "${MOUNTPOINT}/accounting-small.bin" \
  "${MOUNTPOINT}/accounting-large.bin" \
  "${MOUNTPOINT}/accounting-sparse.bin" \
  "${MOUNTPOINT}/accounting-shared-source.bin" \
  "${MOUNTPOINT}/accounting-shared-copy.bin"
sync

echo "OK df/Ph/Phi, unique PostgreSQL payload accounting, shared object attribution, sparse st_blocks, and remount durability"
