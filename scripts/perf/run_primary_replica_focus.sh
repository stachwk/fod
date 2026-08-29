#!/usr/bin/env bash
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1
#
# Repeat the existing primary/replica performance matrix for the
# performance-sensitive 64 KiB and 512 KiB points and aggregate the results.
# The backend follows QNAP=0/1 by default and can be overridden explicitly.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
FILE_SIZE="${FOD_PRIMARY_REPLICA_FOCUS_FILE_SIZE:-1G}"
BLOCK_SIZES="${FOD_PRIMARY_REPLICA_FOCUS_BLOCK_SIZES:-64k 512k}"
REPEAT="${FOD_PRIMARY_REPLICA_FOCUS_REPEAT:-3}"
REQUIRE_AC="${FOD_REQUIRE_AC_POWER:-1}"
ALLOW_DIRTY="${FOD_ALLOW_DIRTY_BENCHMARK:-0}"
BACKEND="${FOD_PRIMARY_REPLICA_FOCUS_BACKEND:-auto}"
PRECHECK_ONLY="${FOD_PRIMARY_REPLICA_FOCUS_PRECHECK_ONLY:-0}"
RUN_ID="$(date -u +%Y%m%dT%H%M%SZ)"
HEAD_SHORT="$(git -C "${ROOT}" rev-parse --short HEAD 2>/dev/null || echo unknown)"
HOST_NAME="$(hostname -s 2>/dev/null || hostname)"
SUMMARIZER="${ROOT}/scripts/perf/summarize_primary_replica_focus.py"
EXTRACTOR="${ROOT}/scripts/perf/extract_primary_replica_profile.sh"

truthy() {
    [[ "${1:-0}" =~ ^(1|true|True|yes|on)$ ]]
}

case "${BACKEND}" in
    auto)
        if truthy "${QNAP:-0}"; then
            BACKEND="qnap"
        else
            BACKEND="local"
        fi
        ;;
    local|qnap)
        ;;
    *)
        echo "FOD_PRIMARY_REPLICA_FOCUS_BACKEND must be auto, local or qnap; got: ${BACKEND}" >&2
        exit 2
        ;;
esac

if [[ "${BACKEND}" == "qnap" ]]; then
    MAKE_TARGET="test-fio-primary-write-replica-read-qnap"
    QNAP_VALUE=1
else
    MAKE_TARGET="test-fio-primary-write-replica-read-matrix"
    QNAP_VALUE=0
fi

ARTIFACT_DIR="${ROOT}/artifacts/perf/${HEAD_SHORT}/${HOST_NAME}-${BACKEND}-focus-${RUN_ID}"

if ! [[ "${REPEAT}" =~ ^[1-9][0-9]*$ ]]; then
    echo "FOD_PRIMARY_REPLICA_FOCUS_REPEAT must be a positive integer, got: ${REPEAT}" >&2
    exit 2
fi

for cmd in git make python3; do
    command -v "${cmd}" >/dev/null 2>&1 || {
        echo "Missing required command: ${cmd}" >&2
        exit 2
    }
done

echo "=== FOD PRIMARY/REPLICA FOCUS SUITE ==="
echo "commit=${HEAD_SHORT}"
echo "host=${HOST_NAME}"
echo "backend=${BACKEND}"
echo "make_target=${MAKE_TARGET}"
echo "file_size=${FILE_SIZE}"
echo "block_sizes=${BLOCK_SIZES}"
echo "repeat=${REPEAT}"
echo "require_ac=${REQUIRE_AC}"
echo "artifact_dir=${ARTIFACT_DIR}"

if truthy "${PRECHECK_ONLY}"; then
    echo "precheck_only=1"
    exit 0
fi

if ! truthy "${ALLOW_DIRTY}" && [[ -n "$(git -C "${ROOT}" status --porcelain)" ]]; then
    echo "Refusing performance run with a dirty Git tree." >&2
    echo "Commit/stash changes or set FOD_ALLOW_DIRTY_BENCHMARK=1 explicitly." >&2
    exit 2
fi

mkdir -p "${ARTIFACT_DIR}"
RUN_INDEX="${ARTIFACT_DIR}/runs.tsv"
printf 'run\tbackend\tmatrix_artifact_dir\tsummary_tsv\tprofile_extract\n' >"${RUN_INDEX}"

for run in $(seq 1 "${REPEAT}"); do
    log="${ARTIFACT_DIR}/run-${run}.log"
    copied_summary="${ARTIFACT_DIR}/summary-run-${run}.tsv"
    profile_extract="${ARTIFACT_DIR}/profile-run-${run}.txt"

    echo
    echo "=== RUN ${run}/${REPEAT} backend=${BACKEND} ==="

    make_args=(
        -C "${ROOT}"
        --no-print-directory
        "QNAP=${QNAP_VALUE}"
        "FOD_REQUIRE_AC_POWER=${REQUIRE_AC}"
        "POSTGRES_PASSWORD_BASE=${POSTGRES_PASSWORD:-password}"
    )
    if [[ "${BACKEND}" == "qnap" ]]; then
        make_args+=(
            "QNAP_REPLICA_READ_FIO_FILE_SIZE=${FILE_SIZE}"
            "QNAP_REPLICA_READ_FIO_BLOCK_SIZES=${BLOCK_SIZES}"
        )
    else
        make_args+=(
            "REPLICA_READ_FIO_FILE_SIZE=${FILE_SIZE}"
            "REPLICA_READ_FIO_BLOCK_SIZES=${BLOCK_SIZES}"
        )
    fi

    set +e
    make "${make_args[@]}" "${MAKE_TARGET}" 2>&1 | tee "${log}"
    status=${PIPESTATUS[0]}
    set -e

    if [[ "${status}" -ne 0 ]]; then
        echo "Primary/replica focus run ${run} failed; backend=${BACKEND} log=${log}" >&2
        exit "${status}"
    fi

    matrix_dir="$(
        sed -n 's/^matrix_artifact_dir=//p' "${log}" | tail -n 1
    )"
    if [[ -z "${matrix_dir}" || ! -f "${matrix_dir}/summary.tsv" ]]; then
        echo "Could not locate matrix summary for run ${run}; log=${log}" >&2
        exit 1
    fi

    cp "${matrix_dir}/summary.tsv" "${copied_summary}"
    "${EXTRACTOR}" "${matrix_dir}" >"${profile_extract}"

    printf '%s\t%s\t%s\t%s\t%s\n' \
        "${run}" "${BACKEND}" "${matrix_dir}" "${copied_summary}" "${profile_extract}" \
        >>"${RUN_INDEX}"
done

python3 "${SUMMARIZER}" \
    --output-tsv "${ARTIFACT_DIR}/summary.tsv" \
    --output-markdown "${ARTIFACT_DIR}/summary.md" \
    "${ARTIFACT_DIR}"/summary-run-*.tsv

echo
echo "=== FOCUS RESULT ==="
cat "${ARTIFACT_DIR}/summary.tsv"
echo
echo "summary_markdown=${ARTIFACT_DIR}/summary.md"
echo "runs_index=${RUN_INDEX}"
echo "focus_artifact_dir=${ARTIFACT_DIR}"
