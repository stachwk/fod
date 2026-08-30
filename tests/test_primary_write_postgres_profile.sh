#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SCRIPT="$ROOT/scripts/perf/profile_primary_write_postgres.sh"

out="$(FOD_PG_WRITE_PROFILE_PRECHECK_ONLY=1 "$SCRIPT")"
grep -q '^host=127.0.0.1$' <<<"$out"
grep -q '^port=55441$' <<<"$out"
grep -q '^poll_seconds=0.25$' <<<"$out"
grep -q '^wal_every=4$' <<<"$out"
grep -q '^process_match=/tmp/fod-primary-write\.$' <<<"$out"
grep -q '^precheck_only=1$' <<<"$out"

out="$(FOD_PG_WRITE_PROFILE_PRECHECK_ONLY=1 FOD_PG_WRITE_PROFILE_PORT=60001 FOD_PG_WRITE_PROFILE_POLL_SECONDS=0.5 FOD_PG_WRITE_PROFILE_WAL_EVERY=2 FOD_PG_WRITE_PROFILE_PROCESS_MATCH='/tmp/custom-primary.' "$SCRIPT")"
grep -q '^port=60001$' <<<"$out"
grep -q '^poll_seconds=0.5$' <<<"$out"
grep -q '^wal_every=2$' <<<"$out"
grep -q '^process_match=/tmp/custom-primary\.$' <<<"$out"

out="$(FOD_PG_WRITE_PROFILE_PRECHECK_ONLY=1 FOD_PG_WRITE_PROFILE_PROCESS_PATTERN='/tmp/legacy-primary.' "$SCRIPT")"
grep -q '^process_match=/tmp/legacy-primary\.$' <<<"$out"

if FOD_PG_WRITE_PROFILE_PRECHECK_ONLY=1 FOD_PG_WRITE_PROFILE_WAL_EVERY=0 "$SCRIPT" >/dev/null 2>&1; then
    echo 'expected WAL_EVERY=0 to fail' >&2
    exit 1
fi

grep -Fq 'processes="$(pgrep -af fod-rust-fuse 2>/dev/null || true)"' "$SCRIPT"
grep -Fq 'grep -F -- "$PROCESS_MATCH"' "$SCRIPT"

if grep -Fq 'awk -v pattern=' "$SCRIPT"; then
    echo 'runtime PID matching must not use awk regex patterns' >&2
    exit 1
fi

echo 'OK primary-write-postgres-profile'
