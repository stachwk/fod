#!/usr/bin/env python3
from __future__ import annotations

import argparse
import hashlib
import json
import math
import multiprocessing as mp
import os
import queue
import re
import statistics
import time
from pathlib import Path
from typing import Any


def percentile(values: list[float], percent: float) -> float:
    if not values:
        raise ValueError("cannot calculate percentile of an empty list")
    ordered = sorted(values)
    rank = (len(ordered) - 1) * percent / 100.0
    lower = math.floor(rank)
    upper = math.ceil(rank)
    if lower == upper:
        return ordered[lower]
    fraction = rank - lower
    return ordered[lower] * (1.0 - fraction) + ordered[upper] * fraction


def write_all(fd: int, payload: bytes) -> int:
    offset = 0
    calls = 0
    while offset < len(payload):
        written = os.write(fd, payload[offset:])
        if written <= 0:
            raise OSError("write returned no progress")
        offset += written
        calls += 1
    return calls


def large_worker(
    worker_id: int,
    path_text: str,
    iterations: int,
    write_bytes: int,
    start_event: Any,
    ready_queue: Any,
    result_queue: Any,
) -> None:
    fd = -1
    path = Path(path_text)
    byte_value = (worker_id * 29 + 7) % 251
    payload = bytes([byte_value]) * write_bytes
    try:
        fd = os.open(path, os.O_CREAT | os.O_TRUNC | os.O_WRONLY, 0o644)
        ready_queue.put(("large", worker_id))
        if not start_event.wait(timeout=30.0):
            raise TimeoutError("large start event was not released")
        operations = []
        total_write_calls = 0
        for operation_index in range(iterations):
            started_ns = time.monotonic_ns()
            total_write_calls += write_all(fd, payload)
            finished_ns = time.monotonic_ns()
            operations.append(
                {
                    "operation_index": operation_index,
                    "started_ns": started_ns,
                    "finished_ns": finished_ns,
                    "latency_ms": (finished_ns - started_ns) / 1_000_000.0,
                }
            )
        os.fsync(fd)
        os.close(fd)
        fd = -1
        result_queue.put(
            {
                "kind": "large",
                "worker_id": worker_id,
                "byte_value": byte_value,
                "path": path_text,
                "operations": operations,
                "write_calls": total_write_calls,
                "error": None,
            }
        )
    except BaseException as exc:
        if fd >= 0:
            try:
                os.close(fd)
            except OSError:
                pass
        result_queue.put(
            {
                "kind": "large",
                "worker_id": worker_id,
                "path": path_text,
                "operations": [],
                "write_calls": 0,
                "error": f"{type(exc).__name__}: {exc}",
            }
        )


def small_worker(
    worker_id: int,
    path_text: str,
    write_bytes: int,
    start_event: Any,
    ready_queue: Any,
    result_queue: Any,
) -> None:
    fd = -1
    path = Path(path_text)
    byte_value = (worker_id * 17 + 3) % 251
    payload = bytes([byte_value]) * write_bytes
    try:
        fd = os.open(path, os.O_CREAT | os.O_TRUNC | os.O_WRONLY, 0o644)
        ready_queue.put(("small", worker_id))
        if not start_event.wait(timeout=30.0):
            raise TimeoutError("small start event was not released")
        started_ns = time.monotonic_ns()
        write_calls = write_all(fd, payload)
        write_finished_ns = time.monotonic_ns()
        os.fsync(fd)
        finished_ns = time.monotonic_ns()
        os.close(fd)
        fd = -1
        result_queue.put(
            {
                "kind": "small",
                "worker_id": worker_id,
                "byte_value": byte_value,
                "path": path_text,
                "started_ns": started_ns,
                "write_finished_ns": write_finished_ns,
                "finished_ns": finished_ns,
                "latency_ms": (
                    write_finished_ns - started_ns
                ) / 1_000_000.0,
                "total_latency_ms": (
                    finished_ns - started_ns
                ) / 1_000_000.0,
                "fsync_tail_ms": (
                    finished_ns - write_finished_ns
                ) / 1_000_000.0,
                "write_calls": write_calls,
                "error": None,
            }
        )
    except BaseException as exc:
        if fd >= 0:
            try:
                os.close(fd)
            except OSError:
                pass
        result_queue.put(
            {
                "kind": "small",
                "worker_id": worker_id,
                "path": path_text,
                "write_calls": 0,
                "error": f"{type(exc).__name__}: {exc}",
            }
        )


def wait_ready(ready_queue: Any, expected: int, timeout_seconds: float) -> None:
    deadline = time.monotonic() + timeout_seconds
    received = 0
    while received < expected:
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            raise TimeoutError(
                f"worker readiness timeout: expected={expected} actual={received}"
            )
        try:
            ready_queue.get(timeout=remaining)
        except queue.Empty as exc:
            raise TimeoutError(
                f"worker readiness timeout: expected={expected} actual={received}"
            ) from exc
        received += 1


def verify_repeated_byte(path: Path, expected_size: int, byte_value: int) -> str | None:
    if not path.exists():
        return f"file is missing: {path.name}"
    actual_size = path.stat().st_size
    if actual_size != expected_size:
        return (
            f"file size mismatch: file={path.name} "
            f"expected={expected_size} actual={actual_size}"
        )
    expected_digest = hashlib.sha256(bytes([byte_value]) * expected_size).hexdigest()
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        while True:
            chunk = handle.read(1024 * 1024)
            if not chunk:
                break
            digest.update(chunk)
    if digest.hexdigest() != expected_digest:
        return f"file payload digest mismatch: {path.name}"
    return None


def cleanup_run_dir(run_dir: Path) -> None:
    if not run_dir.exists():
        return
    for path in run_dir.iterdir():
        try:
            path.unlink()
        except FileNotFoundError:
            pass
    try:
        run_dir.rmdir()
    except OSError:
        pass


def run_workload(args: argparse.Namespace) -> int:
    mountpoint = Path(args.mountpoint).resolve()
    output = Path(args.output).resolve()
    output.parent.mkdir(parents=True, exist_ok=True)

    if args.large_workers < 2:
        raise SystemExit("large-workers must be at least 2")
    if args.large_iterations < 2:
        raise SystemExit("large-iterations must be at least 2")
    if args.large_write_bytes < 4096:
        raise SystemExit("large-write-bytes must be at least 4096")
    if args.small_files < 1 or args.small_bytes < 1:
        raise SystemExit("small-files and small-bytes must be positive")
    if not 0 <= args.overlap_min_percent <= 100:
        raise SystemExit("overlap-min-percent must be between 0 and 100")

    run_dir = mountpoint / f"fifo-saturation-limit-{args.limit}-run-{args.run_index}"
    run_dir.mkdir(mode=0o755)
    large_paths = [
        run_dir / f"large-{worker_id:02d}.bin"
        for worker_id in range(args.large_workers)
    ]
    small_paths = [
        run_dir / f"small-{worker_id:03d}.bin"
        for worker_id in range(args.small_files)
    ]

    ctx = mp.get_context("fork")
    large_start_event = ctx.Event()
    small_start_event = ctx.Event()
    ready_queue = ctx.Queue()
    result_queue = ctx.Queue()
    processes = []

    for worker_id, path in enumerate(large_paths):
        processes.append(
            ctx.Process(
                target=large_worker,
                args=(
                    worker_id,
                    str(path),
                    args.large_iterations,
                    args.large_write_bytes,
                    large_start_event,
                    ready_queue,
                    result_queue,
                ),
                name=f"fod-large-{worker_id:02d}",
            )
        )
    for worker_id, path in enumerate(small_paths):
        processes.append(
            ctx.Process(
                target=small_worker,
                args=(
                    worker_id,
                    str(path),
                    args.small_bytes,
                    small_start_event,
                    ready_queue,
                    result_queue,
                ),
                name=f"fod-small-{worker_id:03d}",
            )
        )

    errors: list[str] = []
    for process in processes:
        process.start()

    expected_results = len(processes)
    large_release_ns = 0
    small_release_ns = 0
    try:
        wait_ready(ready_queue, expected_results, min(args.timeout_seconds, 30.0))
        large_release_ns = time.monotonic_ns()
        large_start_event.set()
        time.sleep(args.injection_delay_ms / 1000.0)
        small_release_ns = time.monotonic_ns()
        small_start_event.set()

        deadline = time.monotonic() + args.timeout_seconds
        for process in processes:
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                break
            process.join(timeout=remaining)

        alive = [process for process in processes if process.is_alive()]
        for process in alive:
            process.terminate()
        for process in alive:
            process.join(timeout=5.0)
        if alive:
            errors.append(
                "processes exceeded timeout: "
                + ",".join(process.name for process in alive)
            )
    except BaseException as exc:
        large_start_event.set()
        small_start_event.set()
        for process in processes:
            if process.is_alive():
                process.terminate()
        for process in processes:
            process.join(timeout=5.0)
        errors.append(f"harness: {type(exc).__name__}: {exc}")

    results = []
    drain_deadline = time.monotonic() + 5.0
    while len(results) < expected_results and time.monotonic() < drain_deadline:
        try:
            results.append(result_queue.get(timeout=0.2))
        except queue.Empty:
            pass

    if len(results) != expected_results:
        errors.append(
            f"worker result count mismatch: expected={expected_results} "
            f"actual={len(results)}"
        )

    for process in processes:
        if process.exitcode not in (0, None):
            errors.append(
                f"worker exit failure: name={process.name} exitcode={process.exitcode}"
            )

    large_results = sorted(
        (item for item in results if item.get("kind") == "large"),
        key=lambda item: int(item["worker_id"]),
    )
    small_results = sorted(
        (item for item in results if item.get("kind") == "small"),
        key=lambda item: int(item["worker_id"]),
    )
    for item in results:
        if item.get("error"):
            errors.append(
                f"{item.get('kind')}-{item.get('worker_id')}: {item['error']}"
            )

    if len(large_results) != args.large_workers:
        errors.append(
            f"large result count mismatch: expected={args.large_workers} "
            f"actual={len(large_results)}"
        )
    if len(small_results) != args.small_files:
        errors.append(
            f"small result count mismatch: expected={args.small_files} "
            f"actual={len(small_results)}"
        )

    large_expected_size = args.large_iterations * args.large_write_bytes
    for item in large_results:
        if item.get("error"):
            continue
        integrity_error = verify_repeated_byte(
            Path(item["path"]), large_expected_size, int(item["byte_value"])
        )
        if integrity_error:
            errors.append(integrity_error)
    for item in small_results:
        if item.get("error"):
            continue
        integrity_error = verify_repeated_byte(
            Path(item["path"]), args.small_bytes, int(item["byte_value"])
        )
        if integrity_error:
            errors.append(integrity_error)

    large_operations = [
        operation
        for item in large_results
        for operation in item.get("operations", [])
    ]
    valid_small = [
        item
        for item in small_results
        if (
            item.get("error") is None
            and "write_finished_ns" in item
            and "finished_ns" in item
        )
    ]

    if large_operations:
        large_started_ns = min(int(item["started_ns"]) for item in large_operations)
        large_finished_ns = max(int(item["finished_ns"]) for item in large_operations)
        large_duration_ms = (
            large_finished_ns - large_started_ns
        ) / 1_000_000.0
    else:
        large_finished_ns = large_release_ns
        large_duration_ms = 0.0
        errors.append("no completed large operations")

    tail_large_operations = sum(
        1
        for operation in large_operations
        if int(operation["finished_ns"]) > small_release_ns
    )
    if tail_large_operations < args.minimum_tail_large_operations:
        errors.append(
            f"insufficient large-operation tail after small release: "
            f"minimum={args.minimum_tail_large_operations} "
            f"actual={tail_large_operations}"
        )

    completed_before_large_end = sum(
        1
        for item in valid_small
        if int(item["write_finished_ns"]) <= large_finished_ns
    )
    overlap_percent = (
        completed_before_large_end * 100.0 / args.small_files
        if args.small_files
        else 0.0
    )
    if args.limit > 0 and overlap_percent < args.overlap_min_percent:
        errors.append(
            f"positive admission limit failed write-progress criterion: "
            f"limit={args.limit} minimum_percent={args.overlap_min_percent} "
            f"actual_percent={overlap_percent:.2f}"
        )

    latencies = [float(item["latency_ms"]) for item in valid_small]
    total_latencies = [
        float(item["total_latency_ms"]) for item in valid_small
    ]
    fsync_tails = [
        float(item["fsync_tail_ms"]) for item in valid_small
    ]
    result: dict[str, Any] = {
        "schema_version": 3,
        "limit": args.limit,
        "run_index": args.run_index,
        "large_workers": args.large_workers,
        "large_iterations": args.large_iterations,
        "large_write_bytes": args.large_write_bytes,
        "large_total_bytes": (
            args.large_workers * args.large_iterations * args.large_write_bytes
        ),
        "large_operation_count": len(large_operations),
        "large_write_syscall_count": sum(
            int(item.get("write_calls", 0)) for item in large_results
        ),
        "large_duration_ms": round(large_duration_ms, 3),
        "large_operations_after_small_release": tail_large_operations,
        "small_files": args.small_files,
        "small_bytes": args.small_bytes,
        "small_completed": len(valid_small),
        "small_write_completed_before_large_end": completed_before_large_end,
        "small_completed_before_large_end": completed_before_large_end,
        "small_overlap_percent": round(overlap_percent, 2),
        "small_latency_min_ms": round(min(latencies), 3) if latencies else None,
        "small_latency_median_ms": (
            round(statistics.median(latencies), 3) if latencies else None
        ),
        "small_latency_p95_ms": (
            round(percentile(latencies, 95.0), 3) if latencies else None
        ),
        "small_latency_max_ms": round(max(latencies), 3) if latencies else None,
        "small_total_latency_median_ms": (
            round(statistics.median(total_latencies), 3)
            if total_latencies
            else None
        ),
        "small_total_latency_p95_ms": (
            round(percentile(total_latencies, 95.0), 3)
            if total_latencies
            else None
        ),
        "small_fsync_tail_median_ms": (
            round(statistics.median(fsync_tails), 3)
            if fsync_tails
            else None
        ),
        "large_release_ns": large_release_ns,
        "small_release_ns": small_release_ns,
        "observability": None,
        "errors": errors,
    }
    output.write_text(
        json.dumps(result, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    print(
        "FOD saturated FIFO FUSE fairness:"
        f" limit={args.limit}"
        f" run={args.run_index}"
        f" large_ms={large_duration_ms:.3f}"
        f" tail_large_ops={tail_large_operations}"
        f" small_median_ms={result['small_latency_median_ms']}"
        f" small_p95_ms={result['small_latency_p95_ms']}"
        f" write_overlap_percent={overlap_percent:.2f}"
        f" errors={len(errors)}"
    )

    cleanup_run_dir(run_dir)
    return 1 if errors else 0


def parse_shutdown_write_observability(log_path: Path) -> dict[str, int]:
    pattern = re.compile(r"([a-z_]+)=([^\s]+)")
    selected: dict[str, str] | None = None
    for line in log_path.read_text(encoding="utf-8", errors="replace").splitlines():
        if "FOD logical task observability:" not in line:
            continue
        fields = dict(pattern.findall(line))
        if (
            fields.get("stage") == "shutdown"
            and fields.get("lane") == "write"
            and fields.get("operation") == "file-write"
        ):
            selected = fields
    if selected is None:
        raise ValueError("shutdown write observability line is missing")

    result = {}
    for key in [
        "admitted_tasks",
        "completed_tasks",
        "failed_tasks",
        "queued_tasks",
        "active_tasks",
        "peak_queued_tasks",
        "peak_active_tasks",
        "accounting_errors",
    ]:
        value = selected.get(key)
        if value is None:
            raise ValueError(f"shutdown write observability lacks {key}")
        result[key] = int(value)
    return result


def annotate(args: argparse.Namespace) -> int:
    input_path = Path(args.input).resolve()
    log_path = Path(args.mount_log).resolve()
    row = json.loads(input_path.read_text(encoding="utf-8"))
    errors = list(row.get("errors", []))

    try:
        observability = parse_shutdown_write_observability(log_path)
    except (OSError, ValueError) as exc:
        errors.append(f"observability: {exc}")
        observability = None

    if observability is not None:
        admitted = observability["admitted_tasks"]
        completed = observability["completed_tasks"]
        failed = observability["failed_tasks"]
        queued = observability["queued_tasks"]
        active = observability["active_tasks"]
        peak_queued = observability["peak_queued_tasks"]
        peak_active = observability["peak_active_tasks"]
        accounting_errors = observability["accounting_errors"]

        if admitted <= 0:
            errors.append("observability admitted_tasks is zero")
        if completed != admitted:
            errors.append(
                f"observability completion mismatch: admitted={admitted} "
                f"completed={completed}"
            )
        if failed != 0:
            errors.append(f"observability failed_tasks is non-zero: {failed}")
        if queued != 0 or active != 0:
            errors.append(
                f"observability did not drain: queued={queued} active={active}"
            )
        if accounting_errors != 0:
            errors.append(
                f"observability accounting_errors is non-zero: {accounting_errors}"
            )

        if args.expected_limit == 0:
            if peak_active < args.minimum_baseline_active:
                errors.append(
                    f"unlimited baseline did not create concurrent callbacks: "
                    f"minimum_peak_active={args.minimum_baseline_active} "
                    f"actual={peak_active}"
                )
        else:
            if peak_queued < args.minimum_peak_queued:
                errors.append(
                    f"admission queue was not saturated: "
                    f"minimum_peak_queued={args.minimum_peak_queued} "
                    f"actual={peak_queued}"
                )
            if peak_active != args.expected_limit:
                errors.append(
                    f"active limit was not exercised exactly: "
                    f"expected_peak_active={args.expected_limit} "
                    f"actual={peak_active}"
                )

    row["observability"] = observability
    row["errors"] = errors
    input_path.write_text(
        json.dumps(row, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    print(
        "FOD saturated FIFO observability:"
        f" limit={args.expected_limit}"
        f" peak_queued={None if observability is None else observability['peak_queued_tasks']}"
        f" peak_active={None if observability is None else observability['peak_active_tasks']}"
        f" callbacks={None if observability is None else observability['admitted_tasks']}"
        f" errors={len(errors)}"
    )
    return 1 if errors else 0


def summarize(args: argparse.Namespace) -> int:
    inputs = [Path(value).resolve() for value in args.inputs]
    if not inputs:
        raise SystemExit("no run JSON files supplied")
    rows = [json.loads(path.read_text(encoding="utf-8")) for path in inputs]
    errors = [
        f"{path.name}: {message}"
        for path, row in zip(inputs, rows)
        for message in row.get("errors", [])
    ]
    grouped: dict[int, list[dict[str, Any]]] = {}
    for row in rows:
        grouped.setdefault(int(row["limit"]), []).append(row)

    summary_rows = []
    for limit in sorted(grouped):
        limit_rows = grouped[limit]
        observability_rows = [
            row["observability"]
            for row in limit_rows
            if row.get("observability") is not None
        ]
        summary_rows.append(
            {
                "limit": limit,
                "runs": len(limit_rows),
                "large_duration_median_ms": round(
                    statistics.median(
                        float(row["large_duration_ms"]) for row in limit_rows
                    ),
                    3,
                ),
                "small_latency_median_of_medians_ms": round(
                    statistics.median(
                        float(row["small_latency_median_ms"]) for row in limit_rows
                    ),
                    3,
                ),
                "small_latency_median_p95_ms": round(
                    statistics.median(
                        float(row["small_latency_p95_ms"]) for row in limit_rows
                    ),
                    3,
                ),
                "small_latency_worst_ms": round(
                    max(float(row["small_latency_max_ms"]) for row in limit_rows),
                    3,
                ),
                "small_overlap_min_percent": round(
                    min(float(row["small_overlap_percent"]) for row in limit_rows),
                    2,
                ),
                "tail_large_operations_min": min(
                    int(row["large_operations_after_small_release"])
                    for row in limit_rows
                ),
                "peak_queued_min": min(
                    int(item["peak_queued_tasks"]) for item in observability_rows
                ) if observability_rows else None,
                "peak_queued_max": max(
                    int(item["peak_queued_tasks"]) for item in observability_rows
                ) if observability_rows else None,
                "peak_active_min": min(
                    int(item["peak_active_tasks"]) for item in observability_rows
                ) if observability_rows else None,
                "peak_active_max": max(
                    int(item["peak_active_tasks"]) for item in observability_rows
                ) if observability_rows else None,
                "write_callbacks_median": round(
                    statistics.median(
                        int(item["admitted_tasks"]) for item in observability_rows
                    ),
                    1,
                ) if observability_rows else None,
            }
        )

    output_json = Path(args.output_json).resolve()
    output_md = Path(args.output_md).resolve()
    output_json.parent.mkdir(parents=True, exist_ok=True)
    output_md.parent.mkdir(parents=True, exist_ok=True)
    summary = {"schema_version": 3, "runs": len(rows), "limits": summary_rows, "errors": errors}
    output_json.write_text(json.dumps(summary, indent=2, sort_keys=True) + "\n", encoding="utf-8")

    lines = [
        "# FOD saturated FIFO FUSE fairness summary",
        "",
        "| write limit | runs | large median ms | small write median ms | small write p95 ms | worst small write ms | minimum write overlap % | minimum tail ops | peak queued min-max | peak active min-max | callbacks median |",
        "| ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |",
    ]
    for row in summary_rows:
        lines.append(
            f"| {row['limit']} | {row['runs']} | "
            f"{row['large_duration_median_ms']:.3f} | "
            f"{row['small_latency_median_of_medians_ms']:.3f} | "
            f"{row['small_latency_median_p95_ms']:.3f} | "
            f"{row['small_latency_worst_ms']:.3f} | "
            f"{row['small_overlap_min_percent']:.2f} | "
            f"{row['tail_large_operations_min']} | "
            f"{row['peak_queued_min']}-{row['peak_queued_max']} | "
            f"{row['peak_active_min']}-{row['peak_active_max']} | "
            f"{row['write_callbacks_median']} |"
        )
    lines.extend([
        "",
        "Positive limits pass only when each run creates a real queued backlog,",
        "reaches the configured active-task limit, drains without accounting",
        "errors, preserves file integrity, and completes the required share",
        "of small write() calls before the continuing large-write stream ends.",
        "fsync latency is recorded separately and is not an admission criterion.",
        "",
    ])
    if errors:
        lines.extend(["## Errors", ""])
        lines.extend(f"- {message}" for message in errors)
        lines.append("")
    output_md.write_text("\n".join(lines), encoding="utf-8")
    print(output_md.read_text(encoding="utf-8"), end="")
    return 1 if errors else 0


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser()
    subparsers = parser.add_subparsers(dest="command", required=True)

    run_parser = subparsers.add_parser("run")
    run_parser.add_argument("--mountpoint", required=True)
    run_parser.add_argument("--output", required=True)
    run_parser.add_argument("--limit", type=int, required=True)
    run_parser.add_argument("--run-index", type=int, required=True)
    run_parser.add_argument("--large-workers", type=int, default=8)
    run_parser.add_argument("--large-iterations", type=int, default=4)
    run_parser.add_argument("--large-write-bytes", type=int, default=262144)
    run_parser.add_argument("--small-files", type=int, default=24)
    run_parser.add_argument("--small-bytes", type=int, default=4096)
    run_parser.add_argument("--injection-delay-ms", type=float, default=5.0)
    run_parser.add_argument("--overlap-min-percent", type=float, default=90.0)
    run_parser.add_argument("--minimum-tail-large-operations", type=int, default=8)
    run_parser.add_argument("--timeout-seconds", type=float, default=180.0)

    annotate_parser = subparsers.add_parser("annotate")
    annotate_parser.add_argument("--input", required=True)
    annotate_parser.add_argument("--mount-log", required=True)
    annotate_parser.add_argument("--expected-limit", type=int, required=True)
    annotate_parser.add_argument("--minimum-peak-queued", type=int, default=2)
    annotate_parser.add_argument("--minimum-baseline-active", type=int, default=2)

    summary_parser = subparsers.add_parser("summarize")
    summary_parser.add_argument("--output-json", required=True)
    summary_parser.add_argument("--output-md", required=True)
    summary_parser.add_argument("inputs", nargs="+")
    return parser


def main() -> int:
    args = build_parser().parse_args()
    if args.command == "run":
        return run_workload(args)
    if args.command == "annotate":
        return annotate(args)
    if args.command == "summarize":
        return summarize(args)
    raise AssertionError(f"unsupported command: {args.command}")


if __name__ == "__main__":
    raise SystemExit(main())
