#!/usr/bin/env bash
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1
#
# Regression test for primary/replica focus backend selection.
# Does not start Docker or execute a benchmark.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
RUNNER="${ROOT}/scripts/perf/run_primary_replica_focus.sh"

check_case() {
    local label="$1"
    local qnap="$2"
    local backend_override="$3"
    local expected_backend="$4"
    local expected_target="$5"
    local output

    if [[ -n "${backend_override}" ]]; then
        output="$(
            QNAP="${qnap}" \
            FOD_PRIMARY_REPLICA_FOCUS_BACKEND="${backend_override}" \
            FOD_PRIMARY_REPLICA_FOCUS_PRECHECK_ONLY=1 \
            bash "${RUNNER}"
        )"
    else
        output="$(
            QNAP="${qnap}" \
            FOD_PRIMARY_REPLICA_FOCUS_PRECHECK_ONLY=1 \
            bash "${RUNNER}"
        )"
    fi

    grep -Fxq "backend=${expected_backend}" <<<"${output}" || {
        echo "${label}: expected backend=${expected_backend}" >&2
        printf '%s\n' "${output}" >&2
        exit 1
    }
    grep -Fxq "make_target=${expected_target}" <<<"${output}" || {
        echo "${label}: expected make_target=${expected_target}" >&2
        printf '%s\n' "${output}" >&2
        exit 1
    }
}

check_case \
    "auto-local" \
    0 \
    "" \
    local \
    test-fio-primary-write-replica-read-matrix

check_case \
    "auto-qnap" \
    1 \
    "" \
    qnap \
    test-fio-primary-write-replica-read-qnap

check_case \
    "explicit-local" \
    1 \
    local \
    local \
    test-fio-primary-write-replica-read-matrix

check_case \
    "explicit-qnap" \
    0 \
    qnap \
    qnap \
    test-fio-primary-write-replica-read-qnap

set +e
invalid_output="$(
    FOD_PRIMARY_REPLICA_FOCUS_BACKEND=invalid \
    FOD_PRIMARY_REPLICA_FOCUS_PRECHECK_ONLY=1 \
    bash "${RUNNER}" 2>&1
)"
invalid_status=$?
set -e

if [[ "${invalid_status}" -ne 2 ]]; then
    echo "invalid-backend: expected exit status 2, got ${invalid_status}" >&2
    printf '%s\n' "${invalid_output}" >&2
    exit 1
fi

grep -Fq 'must be auto, local or qnap' <<<"${invalid_output}" || {
    echo "invalid-backend: expected validation message" >&2
    printf '%s\n' "${invalid_output}" >&2
    exit 1
}

echo "OK primary-replica-focus-backend"
