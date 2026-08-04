#!/usr/bin/env bash
set -euo pipefail

MODE="${1:-all}"
REPEAT="${FIFO_PROFILE_REPEAT:-5}"
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
CARGO_BIN="${RUST_CARGO:-cargo}"
PYTHON_BIN="${PYTHON:-python3}"
RUSTFLAGS_VALUE="${RUSTFLAGS:--D warnings}"
RUN_ID="${FIFO_PROFILE_RUN_ID:-$(date -u +%Y%m%dT%H%M%SZ)}"
COMMIT_SHORT="$(git -C "$ROOT_DIR" rev-parse --short HEAD 2>/dev/null || echo unknown)"
ARTIFACT_DIR="${FIFO_PROFILE_ARTIFACT_DIR:-/tmp/fod-fifo-admission-profile/${COMMIT_SHORT}-${RUN_ID}}"
BENCHMARK_FILTER="logical_task_admission_tests::fifo_notify_all_benchmark_500_waiters"
BUILD_JSON="$ARTIFACT_DIR/cargo-test-build.jsonl"
BASELINE_LOG="$ARTIFACT_DIR/fifo-admission-baseline.log"
BASELINE_SUMMARY="$ARTIFACT_DIR/fifo-admission-baseline-summary.txt"
STRACE_SUMMARY="$ARTIFACT_DIR/fifo-admission-strace-summary.txt"
STRACE_RUN_LOG="$ARTIFACT_DIR/fifo-admission-strace-run.log"
PROFILE_SUMMARY="$ARTIFACT_DIR/fifo-admission-profile-summary.txt"
PROFILE_RUN_LOG="$ARTIFACT_DIR/fifo-admission-profile-run.log"

case "$REPEAT" in
    ''|*[!0-9]*)
        echo "FIFO_PROFILE_REPEAT must be a positive integer, got: $REPEAT" >&2
        exit 1
        ;;
esac
if [ "$REPEAT" -lt 1 ]; then
    echo "FIFO_PROFILE_REPEAT must be at least 1, got: $REPEAT" >&2
    exit 1
fi

case "$MODE" in
    baseline|strace|profile|all) ;;
    *)
        echo "usage: $0 [baseline|strace|profile|all]" >&2
        exit 2
        ;;
esac

mkdir -p "$ARTIFACT_DIR"
cd "$ROOT_DIR"

echo "Building the monitor unit-test binary once..."
RUSTFLAGS="$RUSTFLAGS_VALUE" \
"$CARGO_BIN" test --locked -p fod-rust-monitor --lib --no-run \
    --message-format=json >"$BUILD_JSON"

TEST_BIN="$(
    "$PYTHON_BIN" - "$BUILD_JSON" <<'PY'
import json
import sys
from pathlib import Path

build_log = Path(sys.argv[1])
matches = []
for raw_line in build_log.read_text(encoding="utf-8").splitlines():
    try:
        item = json.loads(raw_line)
    except json.JSONDecodeError:
        continue
    if item.get("reason") != "compiler-artifact":
        continue
    target = item.get("target") or {}
    profile = item.get("profile") or {}
    executable = item.get("executable")
    if (
        executable
        and target.get("name") == "fod_rust_monitor"
        and "lib" in (target.get("kind") or [])
        and profile.get("test") is True
    ):
        matches.append(executable)

if not matches:
    raise SystemExit("unable to locate fod-rust-monitor unit-test executable")
print(matches[-1])
PY
)"

if [ ! -x "$TEST_BIN" ]; then
    echo "test binary is not executable: $TEST_BIN" >&2
    exit 1
fi

run_benchmark() {
    "$TEST_BIN" "$BENCHMARK_FILTER" \
        --ignored --nocapture --test-threads=1
}

summarize_baseline() {
    "$PYTHON_BIN" - "$BASELINE_LOG" "$BASELINE_SUMMARY" <<'PY'
import re
import statistics
import sys
from pathlib import Path

source = Path(sys.argv[1])
destination = Path(sys.argv[2])
pattern = re.compile(
    r"waiters=(?P<waiters>\d+)\s+"
    r"elapsed_us=(?P<elapsed>\d+)\s+"
    r"nanos_per_waiter=(?P<nanos>\d+)"
)
rows = []
for match in pattern.finditer(source.read_text(encoding="utf-8")):
    rows.append(
        (
            int(match.group("waiters")),
            int(match.group("elapsed")),
            int(match.group("nanos")),
        )
    )

if not rows:
    raise SystemExit("no FIFO benchmark measurements found")

waiter_counts = {row[0] for row in rows}
if len(waiter_counts) != 1:
    raise SystemExit(f"inconsistent waiter counts: {sorted(waiter_counts)}")

elapsed = [row[1] for row in rows]
nanos = [row[2] for row in rows]
elapsed_mean = statistics.mean(elapsed)
nanos_mean = statistics.mean(nanos)
elapsed_cv = (
    statistics.pstdev(elapsed) / elapsed_mean * 100.0
    if len(elapsed) > 1 and elapsed_mean
    else 0.0
)

lines = [
    "FOD FIFO admission baseline summary",
    f"waiters={rows[0][0]}",
    f"runs={len(rows)}",
    "elapsed_us=" + ",".join(str(value) for value in elapsed),
    f"elapsed_us_min={min(elapsed)}",
    f"elapsed_us_max={max(elapsed)}",
    f"elapsed_us_mean={elapsed_mean:.1f}",
    f"elapsed_us_median={statistics.median(elapsed):.1f}",
    f"elapsed_us_population_cv_percent={elapsed_cv:.2f}",
    "nanos_per_waiter=" + ",".join(str(value) for value in nanos),
    f"nanos_per_waiter_min={min(nanos)}",
    f"nanos_per_waiter_max={max(nanos)}",
    f"nanos_per_waiter_mean={nanos_mean:.1f}",
    f"nanos_per_waiter_median={statistics.median(nanos):.1f}",
]
destination.write_text("\n".join(lines) + "\n", encoding="utf-8")
PY
    cat "$BASELINE_SUMMARY"
}

run_baseline() {
    : >"$BASELINE_LOG"
    for run_number in $(seq 1 "$REPEAT"); do
        {
            echo "=== FIFO benchmark run $run_number/$REPEAT ==="
            run_benchmark
            echo
        } 2>&1 | tee -a "$BASELINE_LOG"
    done
    summarize_baseline
}

run_strace() {
    command -v strace >/dev/null 2>&1 || {
        echo "strace is required for FIFO admission syscall profiling" >&2
        exit 1
    }
    echo "Running strace syscall summary..."
    strace -f -c -o "$STRACE_SUMMARY" \
        "$TEST_BIN" "$BENCHMARK_FILTER" \
        --ignored --nocapture --test-threads=1 \
        >"$STRACE_RUN_LOG" 2>&1
    cat "$STRACE_RUN_LOG"
    echo
    echo "FOD FIFO admission strace summary:"
    cat "$STRACE_SUMMARY"
}

run_profile() {
    : >"$PROFILE_RUN_LOG"
    if command -v perf >/dev/null 2>&1 \
        && perf stat -e task-clock -- true >/dev/null 2>&1; then
        echo "Running perf stat profile ($REPEAT repetitions)..."
        perf stat -d -r "$REPEAT" -o "$PROFILE_SUMMARY" -- \
            "$TEST_BIN" "$BENCHMARK_FILTER" \
            --ignored --nocapture --test-threads=1 \
            >"$PROFILE_RUN_LOG" 2>&1
        cat "$PROFILE_RUN_LOG"
        echo
        echo "FOD FIFO admission perf stat summary:"
        cat "$PROFILE_SUMMARY"
        return
    fi

    if [ ! -x /usr/bin/time ]; then
        echo "neither usable perf stat nor /usr/bin/time is available" >&2
        exit 1
    fi

    echo "perf counters unavailable; using /usr/bin/time -v ($REPEAT repetitions)..."
    : >"$PROFILE_SUMMARY"
    for run_number in $(seq 1 "$REPEAT"); do
        {
            echo "=== resource profile run $run_number/$REPEAT ==="
            /usr/bin/time -v -a -o "$PROFILE_SUMMARY" \
                "$TEST_BIN" "$BENCHMARK_FILTER" \
                --ignored --nocapture --test-threads=1
            echo
        } >>"$PROFILE_RUN_LOG" 2>&1
    done
    cat "$PROFILE_RUN_LOG"
    echo
    echo "FOD FIFO admission resource profile summary:"
    cat "$PROFILE_SUMMARY"
}

case "$MODE" in
    baseline) run_baseline ;;
    strace) run_strace ;;
    profile) run_profile ;;
    all)
        run_baseline
        run_strace
        run_profile
        ;;
esac

echo
echo "FIFO admission profiling artifacts: $ARTIFACT_DIR"
