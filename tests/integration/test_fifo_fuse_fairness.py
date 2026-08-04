#!/usr/bin/env python3
from __future__ import annotations

import argparse
import json
import math
import os
import statistics
import threading
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


def write_all(fd: int, payload: bytes) -> None:
    offset = 0
    while offset < len(payload):
        written = os.write(fd, payload[offset:])
        if written <= 0:
            raise OSError("write returned no progress")
        offset += written


def run_workload(args: argparse.Namespace) -> int:
    mountpoint = Path(args.mountpoint).resolve()
    output = Path(args.output).resolve()
    output.parent.mkdir(parents=True, exist_ok=True)

    if args.large_bytes < args.large_chunk_bytes:
        raise SystemExit("large-bytes must be at least large-chunk-bytes")
    if args.large_bytes % args.large_chunk_bytes != 0:
        raise SystemExit("large-bytes must be divisible by large-chunk-bytes")
    if args.small_files < 1:
        raise SystemExit("small-files must be positive")
    if args.small_bytes < 1:
        raise SystemExit("small-bytes must be positive")
    if not 0 <= args.overlap_min_percent <= 100:
        raise SystemExit("overlap-min-percent must be between 0 and 100")

    run_dir = mountpoint / f"fifo-fairness-limit-{args.limit}-run-{args.run_index}"
    run_dir.mkdir(mode=0o755)
    large_path = run_dir / "large.bin"
    small_paths = [run_dir / f"small-{index:03d}.bin" for index in range(args.small_files)]

    large_started = threading.Event()
    large_times: dict[str, int] = {}
    small_results: list[dict[str, Any] | None] = [None] * args.small_files
    errors: list[str] = []
    error_lock = threading.Lock()

    large_payload = bytes([0x5A]) * args.large_chunk_bytes
    small_payloads = [
        bytes([(index * 17 + 3) % 251]) * args.small_bytes
        for index in range(args.small_files)
    ]
    pace_seconds = args.large_pace_ms / 1000.0

    def record_error(label: str, exc: BaseException) -> None:
        with error_lock:
            errors.append(f"{label}: {type(exc).__name__}: {exc}")

    def large_writer() -> None:
        fd = -1
        try:
            started_ns = time.monotonic_ns()
            large_times["started_ns"] = started_ns
            fd = os.open(large_path, os.O_CREAT | os.O_TRUNC | os.O_WRONLY, 0o644)
            chunks = args.large_bytes // args.large_chunk_bytes
            for chunk_index in range(chunks):
                write_all(fd, large_payload)
                if chunk_index == 0:
                    large_started.set()
                if pace_seconds > 0:
                    time.sleep(pace_seconds)
            os.fsync(fd)
            os.close(fd)
            fd = -1
            large_times["finished_ns"] = time.monotonic_ns()
        except BaseException as exc:  # test harness must surface worker failures
            large_started.set()
            record_error("large-writer", exc)
            if fd >= 0:
                try:
                    os.close(fd)
                except OSError:
                    pass

    def small_writer(index: int) -> None:
        fd = -1
        started_ns = time.monotonic_ns()
        try:
            fd = os.open(
                small_paths[index],
                os.O_CREAT | os.O_TRUNC | os.O_WRONLY,
                0o644,
            )
            write_all(fd, small_payloads[index])
            os.fsync(fd)
            os.close(fd)
            fd = -1
            finished_ns = time.monotonic_ns()
            small_results[index] = {
                "index": index,
                "started_ns": started_ns,
                "finished_ns": finished_ns,
                "latency_ms": (finished_ns - started_ns) / 1_000_000.0,
            }
        except BaseException as exc:
            record_error(f"small-writer-{index}", exc)
            if fd >= 0:
                try:
                    os.close(fd)
                except OSError:
                    pass

    large_thread = threading.Thread(target=large_writer, name="fod-large-writer")
    large_thread.start()
    if not large_started.wait(timeout=min(args.timeout_seconds, 30.0)):
        raise SystemExit("large writer did not submit its first FUSE write")

    time.sleep(args.small_start_delay_ms / 1000.0)
    small_threads = []
    for index in range(args.small_files):
        thread = threading.Thread(
            target=small_writer,
            args=(index,),
            name=f"fod-small-writer-{index:03d}",
        )
        small_threads.append(thread)
        thread.start()
        if args.small_launch_stagger_ms > 0:
            time.sleep(args.small_launch_stagger_ms / 1000.0)

    deadline = time.monotonic() + args.timeout_seconds
    for thread in small_threads:
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            break
        thread.join(timeout=remaining)
    remaining = deadline - time.monotonic()
    if remaining > 0:
        large_thread.join(timeout=remaining)

    alive = [thread.name for thread in [large_thread, *small_threads] if thread.is_alive()]
    if alive:
        errors.append("threads still alive after timeout: " + ",".join(alive))

    completed = [item for item in small_results if item is not None]
    if len(completed) != args.small_files:
        errors.append(
            f"small completion count mismatch: expected={args.small_files} "
            f"actual={len(completed)}"
        )

    if large_path.exists() and large_path.stat().st_size != args.large_bytes:
        errors.append(
            f"large file size mismatch: expected={args.large_bytes} "
            f"actual={large_path.stat().st_size}"
        )
    elif not large_path.exists():
        errors.append("large output file is missing")

    for index, small_path in enumerate(small_paths):
        if not small_path.exists():
            errors.append(f"small file missing: {small_path.name}")
            continue
        size = small_path.stat().st_size
        if size != args.small_bytes:
            errors.append(
                f"small file size mismatch: file={small_path.name} "
                f"expected={args.small_bytes} actual={size}"
            )
            continue
        with small_path.open("rb") as handle:
            actual = handle.read(args.small_bytes)
        if actual != small_payloads[index]:
            errors.append(f"small file payload mismatch: {small_path.name}")

    large_started_ns = large_times.get("started_ns")
    large_finished_ns = large_times.get("finished_ns")
    if large_started_ns is None or large_finished_ns is None:
        errors.append("large writer timing is incomplete")
        large_duration_ms = 0.0
        completed_before_large_end = 0
    else:
        large_duration_ms = (
            large_finished_ns - large_started_ns
        ) / 1_000_000.0
        completed_before_large_end = sum(
            1
            for item in completed
            if int(item["finished_ns"]) <= large_finished_ns
        )

    if large_duration_ms < args.minimum_large_duration_ms:
        errors.append(
            f"large workload overlap window too short: "
            f"minimum_ms={args.minimum_large_duration_ms} "
            f"actual_ms={large_duration_ms:.3f}"
        )

    overlap_percent = (
        completed_before_large_end * 100.0 / args.small_files
        if args.small_files
        else 0.0
    )
    if args.limit > 0 and overlap_percent < args.overlap_min_percent:
        errors.append(
            f"positive admission limit failed overlap criterion: "
            f"limit={args.limit} minimum_percent={args.overlap_min_percent} "
            f"actual_percent={overlap_percent:.2f}"
        )

    latencies = [float(item["latency_ms"]) for item in completed]
    result: dict[str, Any] = {
        "schema_version": 1,
        "limit": args.limit,
        "run_index": args.run_index,
        "large_bytes": args.large_bytes,
        "large_chunk_bytes": args.large_chunk_bytes,
        "large_pace_ms": args.large_pace_ms,
        "small_files": args.small_files,
        "small_bytes": args.small_bytes,
        "large_duration_ms": round(large_duration_ms, 3),
        "small_completed": len(completed),
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
        "errors": errors,
        "small_results": completed,
    }
    output.write_text(json.dumps(result, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(
        "FOD FIFO FUSE fairness:"
        f" limit={args.limit}"
        f" run={args.run_index}"
        f" large_ms={large_duration_ms:.3f}"
        f" small_median_ms={result['small_latency_median_ms']}"
        f" small_p95_ms={result['small_latency_p95_ms']}"
        f" small_max_ms={result['small_latency_max_ms']}"
        f" overlap_percent={overlap_percent:.2f}"
        f" errors={len(errors)}"
    )

    for path in small_paths:
        try:
            path.unlink()
        except FileNotFoundError:
            pass
    try:
        large_path.unlink()
    except FileNotFoundError:
        pass
    try:
        run_dir.rmdir()
    except OSError:
        pass

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
        medians = [float(row["small_latency_median_ms"]) for row in limit_rows]
        p95s = [float(row["small_latency_p95_ms"]) for row in limit_rows]
        maxima = [float(row["small_latency_max_ms"]) for row in limit_rows]
        large_durations = [float(row["large_duration_ms"]) for row in limit_rows]
        overlaps = [float(row["small_overlap_percent"]) for row in limit_rows]
        summary_rows.append(
            {
                "limit": limit,
                "runs": len(limit_rows),
                "large_duration_median_ms": round(
                    statistics.median(large_durations), 3
                ),
                "small_latency_median_of_medians_ms": round(
                    statistics.median(medians), 3
                ),
                "small_latency_median_p95_ms": round(
                    statistics.median(p95s), 3
                ),
                "small_latency_worst_ms": round(max(maxima), 3),
                "small_overlap_min_percent": round(min(overlaps), 2),
                "small_overlap_median_percent": round(
                    statistics.median(overlaps), 2
                ),
            }
        )

    output_json = Path(args.output_json).resolve()
    output_md = Path(args.output_md).resolve()
    output_json.parent.mkdir(parents=True, exist_ok=True)
    output_md.parent.mkdir(parents=True, exist_ok=True)

    summary = {
        "schema_version": 1,
        "runs": len(rows),
        "limits": summary_rows,
        "errors": errors,
    }
    output_json.write_text(
        json.dumps(summary, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )

    lines = [
        "# FOD FIFO FUSE fairness summary",
        "",
        "| write limit | runs | large median ms | small median ms | small p95 ms | worst small ms | minimum overlap % |",
        "| ---: | ---: | ---: | ---: | ---: | ---: | ---: |",
    ]
    for row in summary_rows:
        lines.append(
            f"| {row['limit']} | {row['runs']} | "
            f"{row['large_duration_median_ms']:.3f} | "
            f"{row['small_latency_median_of_medians_ms']:.3f} | "
            f"{row['small_latency_median_p95_ms']:.3f} | "
            f"{row['small_latency_worst_ms']:.3f} | "
            f"{row['small_overlap_min_percent']:.2f} |"
        )
    lines.extend(
        [
            "",
            "A positive write admission limit passes only when every run completes",
            "without worker or data-integrity errors and reaches the configured",
            "minimum overlap of small-file completions while the large write is active.",
            "",
        ]
    )
    if errors:
        lines.append("## Errors")
        lines.append("")
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
    run_parser.add_argument("--large-bytes", type=int, default=4 * 1024 * 1024)
    run_parser.add_argument("--large-chunk-bytes", type=int, default=4096)
    run_parser.add_argument("--large-pace-ms", type=float, default=0.5)
    run_parser.add_argument("--minimum-large-duration-ms", type=float, default=300.0)
    run_parser.add_argument("--small-files", type=int, default=24)
    run_parser.add_argument("--small-bytes", type=int, default=4096)
    run_parser.add_argument("--small-start-delay-ms", type=float, default=20.0)
    run_parser.add_argument("--small-launch-stagger-ms", type=float, default=0.25)
    run_parser.add_argument("--overlap-min-percent", type=float, default=90.0)
    run_parser.add_argument("--timeout-seconds", type=float, default=120.0)

    summary_parser = subparsers.add_parser("summarize")
    summary_parser.add_argument("--output-json", required=True)
    summary_parser.add_argument("--output-md", required=True)
    summary_parser.add_argument("inputs", nargs="+")

    return parser


def main() -> int:
    args = build_parser().parse_args()
    if args.command == "run":
        return run_workload(args)
    if args.command == "summarize":
        return summarize(args)
    raise AssertionError(f"unsupported command: {args.command}")


if __name__ == "__main__":
    raise SystemExit(main())
