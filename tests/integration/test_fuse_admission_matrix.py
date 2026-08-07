#!/usr/bin/env python3
from __future__ import annotations

import argparse
import json
import math
import re
import statistics
from pathlib import Path
from typing import Any

from test_fifo_fuse_fairness import parse_shutdown_write_observability


def coefficient_of_variation_percent(values: list[float]) -> float:
    if len(values) < 2:
        return 0.0
    mean = statistics.mean(values)
    if mean == 0:
        return 0.0
    return statistics.pstdev(values) * 100.0 / mean


def parse_event_loop(log_path: Path) -> dict[str, Any]:
    pattern = re.compile(
        r"FOD FUSE event loop: threads=(?P<threads>[0-9]+) "
        r"clone_fd=(?P<clone_fd>true|false)"
    )
    selected = None
    for line in log_path.read_text(encoding="utf-8", errors="replace").splitlines():
        match = pattern.search(line)
        if match:
            selected = {
                "threads": int(match.group("threads")),
                "clone_fd": match.group("clone_fd") == "true",
            }
    if selected is None:
        raise ValueError("FOD FUSE event-loop configuration line is missing")
    return selected


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

    try:
        event_loop = parse_event_loop(log_path)
    except (OSError, ValueError) as exc:
        errors.append(f"event-loop: {exc}")
        event_loop = None

    if event_loop is not None:
        if event_loop["threads"] != args.event_threads:
            errors.append(
                "event-loop thread mismatch: "
                f"expected={args.event_threads} actual={event_loop['threads']}"
            )
        if event_loop["clone_fd"] != args.clone_fd:
            errors.append(
                "event-loop clone_fd mismatch: "
                f"expected={args.clone_fd} actual={event_loop['clone_fd']}"
            )

    binding = args.limit > 0 and args.limit < args.event_threads
    effective_capacity = (
        args.event_threads
        if args.limit == 0
        else min(args.limit, args.event_threads)
    )

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
        if peak_active > args.event_threads:
            errors.append(
                f"peak_active exceeds event threads: "
                f"threads={args.event_threads} peak_active={peak_active}"
            )
        if args.limit > 0 and peak_active > args.limit:
            errors.append(
                f"peak_active exceeds configured admission limit: "
                f"limit={args.limit} peak_active={peak_active}"
            )

        if binding:
            if peak_queued < args.minimum_peak_queued:
                errors.append(
                    f"binding configuration did not create queue pressure: "
                    f"minimum={args.minimum_peak_queued} actual={peak_queued}"
                )
            if peak_active != args.limit:
                errors.append(
                    f"binding admission limit was not fully exercised: "
                    f"expected={args.limit} actual={peak_active}"
                )
        else:
            if peak_active < args.minimum_baseline_active:
                errors.append(
                    f"non-binding configuration did not demonstrate concurrency: "
                    f"minimum={args.minimum_baseline_active} actual={peak_active}"
                )

    large_total_bytes = int(row.get("large_total_bytes", 0))
    large_duration_ms = float(row.get("large_duration_ms", 0.0))
    throughput_mib_s = (
        large_total_bytes / (1024.0 * 1024.0) / (large_duration_ms / 1000.0)
        if large_total_bytes > 0 and large_duration_ms > 0
        else 0.0
    )

    row["matrix"] = {
        "event_threads": args.event_threads,
        "configured_limit": args.limit,
        "effective_capacity": effective_capacity,
        "binding": binding,
        "clone_fd": args.clone_fd,
        "large_throughput_mib_s": round(throughput_mib_s, 3),
    }
    row["observability"] = observability
    row["event_loop"] = event_loop
    row["errors"] = errors
    input_path.write_text(
        json.dumps(row, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )

    print(
        "FOD admission matrix annotation:"
        f" threads={args.event_threads}"
        f" limit={args.limit}"
        f" binding={int(binding)}"
        f" throughput_mib_s={throughput_mib_s:.3f}"
        f" peak_queued={None if observability is None else observability['peak_queued_tasks']}"
        f" peak_active={None if observability is None else observability['peak_active_tasks']}"
        f" errors={len(errors)}"
    )
    return 1 if errors else 0


def score_rows(summary_rows: list[dict[str, Any]]) -> None:
    eligible = [row for row in summary_rows if not row["errors"]]
    if not eligible:
        return

    max_throughput = max(row["throughput_median_mib_s"] for row in eligible)
    min_median = min(row["small_write_median_ms"] for row in eligible)
    min_p95 = min(row["small_write_p95_ms"] for row in eligible)

    for row in eligible:
        throughput_score = (
            row["throughput_median_mib_s"] / max_throughput
            if max_throughput > 0
            else 0.0
        )
        median_score = (
            min_median / row["small_write_median_ms"]
            if row["small_write_median_ms"] > 0
            else 0.0
        )
        p95_score = (
            min_p95 / row["small_write_p95_ms"]
            if row["small_write_p95_ms"] > 0
            else 0.0
        )
        stability_score = 1.0 / (1.0 + row["stability_cv_percent"] / 100.0)

        score = 100.0 * (
            0.35 * throughput_score
            + 0.25 * median_score
            + 0.25 * p95_score
            + 0.15 * stability_score
        )
        row["diagnostic_score"] = round(score, 3)

    ranked = sorted(
        eligible,
        key=lambda row: (
            -float(row["diagnostic_score"]),
            int(row["event_threads"]),
            int(row["limit"]),
        ),
    )
    for rank, row in enumerate(ranked, start=1):
        row["rank"] = rank


def summarize(args: argparse.Namespace) -> int:
    inputs = [Path(value).resolve() for value in args.inputs]
    if not inputs:
        raise SystemExit("no matrix run JSON files supplied")

    rows = [json.loads(path.read_text(encoding="utf-8")) for path in inputs]
    global_errors = [
        f"{path.name}: {message}"
        for path, row in zip(inputs, rows)
        for message in row.get("errors", [])
    ]

    grouped: dict[tuple[int, int], list[dict[str, Any]]] = {}
    for row in rows:
        matrix = row.get("matrix")
        if not matrix:
            global_errors.append("matrix metadata is missing from a run")
            continue
        key = (int(matrix["event_threads"]), int(matrix["configured_limit"]))
        grouped.setdefault(key, []).append(row)

    summary_rows: list[dict[str, Any]] = []
    for (threads, limit), cell_rows in sorted(grouped.items()):
        throughputs = [
            float(row["matrix"]["large_throughput_mib_s"]) for row in cell_rows
        ]
        small_medians = [
            float(row["small_latency_median_ms"]) for row in cell_rows
        ]
        small_p95s = [
            float(row["small_latency_p95_ms"]) for row in cell_rows
        ]
        large_durations = [
            float(row["large_duration_ms"]) for row in cell_rows
        ]
        overlaps = [
            float(row["small_overlap_percent"]) for row in cell_rows
        ]
        observability_rows = [
            row["observability"]
            for row in cell_rows
            if row.get("observability") is not None
        ]
        cell_errors = [
            message
            for row in cell_rows
            for message in row.get("errors", [])
        ]
        binding = bool(cell_rows[0]["matrix"]["binding"])

        cv_values = [
            coefficient_of_variation_percent(throughputs),
            coefficient_of_variation_percent(small_medians),
            coefficient_of_variation_percent(small_p95s),
        ]
        summary_rows.append(
            {
                "event_threads": threads,
                "limit": limit,
                "binding": binding,
                "runs": len(cell_rows),
                "throughput_median_mib_s": round(
                    statistics.median(throughputs), 3
                ),
                "large_duration_median_ms": round(
                    statistics.median(large_durations), 3
                ),
                "small_write_median_ms": round(
                    statistics.median(small_medians), 3
                ),
                "small_write_p95_ms": round(
                    statistics.median(small_p95s), 3
                ),
                "small_write_worst_ms": round(
                    max(float(row["small_latency_max_ms"]) for row in cell_rows),
                    3,
                ),
                "minimum_write_overlap_percent": round(min(overlaps), 2),
                "throughput_cv_percent": round(cv_values[0], 2),
                "small_median_cv_percent": round(cv_values[1], 2),
                "small_p95_cv_percent": round(cv_values[2], 2),
                "stability_cv_percent": round(statistics.mean(cv_values), 2),
                "peak_queued_min": (
                    min(
                        int(item["peak_queued_tasks"])
                        for item in observability_rows
                    )
                    if observability_rows
                    else None
                ),
                "peak_queued_max": (
                    max(
                        int(item["peak_queued_tasks"])
                        for item in observability_rows
                    )
                    if observability_rows
                    else None
                ),
                "peak_active_min": (
                    min(
                        int(item["peak_active_tasks"])
                        for item in observability_rows
                    )
                    if observability_rows
                    else None
                ),
                "peak_active_max": (
                    max(
                        int(item["peak_active_tasks"])
                        for item in observability_rows
                    )
                    if observability_rows
                    else None
                ),
                "errors": cell_errors,
                "diagnostic_score": None,
                "rank": None,
                "throughput_vs_same_threads_unlimited_percent": None,
                "small_median_vs_same_threads_unlimited_percent": None,
                "small_p95_vs_same_threads_unlimited_percent": None,
            }
        )

    baselines = {
        int(row["event_threads"]): row
        for row in summary_rows
        if int(row["limit"]) == 0 and not row["errors"]
    }
    for row in summary_rows:
        baseline = baselines.get(int(row["event_threads"]))
        if baseline is None or row["errors"]:
            continue

        for key, output_key in [
            (
                "throughput_median_mib_s",
                "throughput_vs_same_threads_unlimited_percent",
            ),
            (
                "small_write_median_ms",
                "small_median_vs_same_threads_unlimited_percent",
            ),
            (
                "small_write_p95_ms",
                "small_p95_vs_same_threads_unlimited_percent",
            ),
        ]:
            base = float(baseline[key])
            value = float(row[key])
            if base != 0:
                row[output_key] = round((value - base) * 100.0 / base, 2)

    score_rows(summary_rows)

    eligible = [row for row in summary_rows if not row["errors"]]
    overall_best = min(
        eligible,
        key=lambda row: int(row["rank"]) if row["rank"] is not None else 10**9,
        default=None,
    )
    binding_candidates = [
        row
        for row in eligible
        if row["binding"] and int(row["limit"]) > 0
    ]
    binding_best = min(
        binding_candidates,
        key=lambda row: int(row["rank"]) if row["rank"] is not None else 10**9,
        default=None,
    )

    summary = {
        "schema_version": 1,
        "runs": len(rows),
        "cells": len(summary_rows),
        "errors": global_errors,
        "weights": {
            "throughput": 0.35,
            "small_write_median": 0.25,
            "small_write_p95": 0.25,
            "stability": 0.15,
        },
        "overall_best": overall_best,
        "best_binding_candidate": binding_best,
        "matrix": sorted(
            summary_rows,
            key=lambda row: (
                row["rank"] is None,
                int(row["rank"]) if row["rank"] is not None else 10**9,
                int(row["event_threads"]),
                int(row["limit"]),
            ),
        ),
    }

    output_json = Path(args.output_json).resolve()
    output_md = Path(args.output_md).resolve()
    output_csv = Path(args.output_csv).resolve()
    for path in [output_json, output_md, output_csv]:
        path.parent.mkdir(parents=True, exist_ok=True)

    output_json.write_text(
        json.dumps(summary, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )

    lines = [
        "# FOD FUSE admission tuning matrix",
        "",
        "Diagnostic score weights: throughput 35%, small-write median 25%, "
        "small-write p95 25%, stability 15%. The score is a comparison aid, "
        "not an automatic production-default decision.",
        "",
        "| rank | threads | limit | binding | score | throughput MiB/s | small median ms | small p95 ms | stability CV % | overlap % | peak queued | peak active | throughput vs unlimited % | median vs unlimited % | p95 vs unlimited % |",
        "| ---: | ---: | ---: | :---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |",
    ]
    for row in summary["matrix"]:
        def fmt(value: Any, digits: int = 2) -> str:
            if value is None:
                return "-"
            if isinstance(value, float):
                return f"{value:.{digits}f}"
            return str(value)

        lines.append(
            f"| {fmt(row['rank'], 0)} | {row['event_threads']} | "
            f"{row['limit']} | {'yes' if row['binding'] else 'no'} | "
            f"{fmt(row['diagnostic_score'], 3)} | "
            f"{fmt(row['throughput_median_mib_s'], 3)} | "
            f"{fmt(row['small_write_median_ms'], 3)} | "
            f"{fmt(row['small_write_p95_ms'], 3)} | "
            f"{fmt(row['stability_cv_percent'])} | "
            f"{fmt(row['minimum_write_overlap_percent'])} | "
            f"{fmt(row['peak_queued_min'], 0)}-{fmt(row['peak_queued_max'], 0)} | "
            f"{fmt(row['peak_active_min'], 0)}-{fmt(row['peak_active_max'], 0)} | "
            f"{fmt(row['throughput_vs_same_threads_unlimited_percent'])} | "
            f"{fmt(row['small_median_vs_same_threads_unlimited_percent'])} | "
            f"{fmt(row['small_p95_vs_same_threads_unlimited_percent'])} |"
        )

    if overall_best is not None:
        lines.extend(
            [
                "",
                "## Ranking result",
                "",
                f"Overall diagnostic leader: threads={overall_best['event_threads']}, "
                f"limit={overall_best['limit']}, "
                f"score={overall_best['diagnostic_score']:.3f}.",
            ]
        )
    if binding_best is not None:
        lines.append(
            f"Best binding admission candidate: threads={binding_best['event_threads']}, "
            f"limit={binding_best['limit']}, "
            f"score={binding_best['diagnostic_score']:.3f}."
        )

    if global_errors:
        lines.extend(["", "## Errors", ""])
        lines.extend(f"- {message}" for message in global_errors)

    output_md.write_text("\n".join(lines) + "\n", encoding="utf-8")

    csv_columns = [
        "rank",
        "event_threads",
        "limit",
        "binding",
        "diagnostic_score",
        "throughput_median_mib_s",
        "small_write_median_ms",
        "small_write_p95_ms",
        "stability_cv_percent",
        "minimum_write_overlap_percent",
        "peak_queued_min",
        "peak_queued_max",
        "peak_active_min",
        "peak_active_max",
        "throughput_vs_same_threads_unlimited_percent",
        "small_median_vs_same_threads_unlimited_percent",
        "small_p95_vs_same_threads_unlimited_percent",
    ]
    csv_lines = [",".join(csv_columns)]
    for row in summary["matrix"]:
        csv_lines.append(
            ",".join(str(row.get(column, "")) for column in csv_columns)
        )
    output_csv.write_text("\n".join(csv_lines) + "\n", encoding="utf-8")

    print(output_md.read_text(encoding="utf-8"), end="")
    return 1 if global_errors else 0


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser()
    subparsers = parser.add_subparsers(dest="command", required=True)

    annotate_parser = subparsers.add_parser("annotate")
    annotate_parser.add_argument("--input", required=True)
    annotate_parser.add_argument("--mount-log", required=True)
    annotate_parser.add_argument("--event-threads", type=int, required=True)
    annotate_parser.add_argument("--limit", type=int, required=True)
    annotate_parser.add_argument("--clone-fd", action="store_true")
    annotate_parser.add_argument("--minimum-peak-queued", type=int, default=2)
    annotate_parser.add_argument("--minimum-baseline-active", type=int, default=2)

    summarize_parser = subparsers.add_parser("summarize")
    summarize_parser.add_argument("--output-json", required=True)
    summarize_parser.add_argument("--output-md", required=True)
    summarize_parser.add_argument("--output-csv", required=True)
    summarize_parser.add_argument("inputs", nargs="+")

    return parser


def main() -> int:
    args = build_parser().parse_args()
    if args.command == "annotate":
        return annotate(args)
    if args.command == "summarize":
        return summarize(args)
    raise AssertionError(f"unsupported command: {args.command}")


if __name__ == "__main__":
    raise SystemExit(main())
