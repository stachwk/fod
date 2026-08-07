#!/usr/bin/env python3
from __future__ import annotations

import argparse
import json
from pathlib import Path
from typing import Any


REQUIRED_CANDIDATES = ((8, 4), (16, 4), (4, 8), (8, 0))
TARGET = (8, 4)
BASELINE = (8, 0)


def find_cell(summary: dict[str, Any], threads: int, limit: int) -> dict[str, Any]:
    for row in summary.get("matrix", []):
        if int(row["event_threads"]) == threads and int(row["limit"]) == limit:
            return row
    raise ValueError(f"missing candidate threads={threads} limit={limit}")


def improvement_percent(lower_is_better_value: float, baseline_value: float) -> float:
    if baseline_value == 0:
        return 0.0
    return (baseline_value - lower_is_better_value) * 100.0 / baseline_value


def throughput_gain_percent(value: float, baseline_value: float) -> float:
    if baseline_value == 0:
        return 0.0
    return (value - baseline_value) * 100.0 / baseline_value


def report(args: argparse.Namespace) -> int:
    matrix_path = Path(args.matrix_summary).resolve()
    output_json = Path(args.output_json).resolve()
    output_md = Path(args.output_md).resolve()
    fio_dir = Path(args.fio_artifact_dir).resolve()

    summary = json.loads(matrix_path.read_text(encoding="utf-8"))
    errors = list(summary.get("errors", []))
    cells: dict[tuple[int, int], dict[str, Any]] = {}

    for pair in REQUIRED_CANDIDATES:
        try:
            cell = find_cell(summary, *pair)
        except ValueError as exc:
            errors.append(str(exc))
            continue
        cells[pair] = cell
        if int(cell.get("runs", 0)) != args.expected_runs:
            errors.append(
                f"candidate {pair[0]}/{pair[1]} run count mismatch: "
                f"expected={args.expected_runs} actual={cell.get('runs')}"
            )
        if cell.get("errors"):
            errors.extend(
                f"candidate {pair[0]}/{pair[1]}: {message}"
                for message in cell["errors"]
            )
        if float(cell.get("minimum_write_overlap_percent", 0.0)) < 100.0:
            errors.append(
                f"candidate {pair[0]}/{pair[1]} did not keep 100% write overlap"
            )

    if errors:
        verdict = "invalid"
        checks: dict[str, bool] = {}
        metrics: dict[str, float | int | None] = {}
    else:
        target = cells[TARGET]
        baseline = cells[BASELINE]

        throughput_gain = throughput_gain_percent(
            float(target["throughput_median_mib_s"]),
            float(baseline["throughput_median_mib_s"]),
        )
        median_improvement = improvement_percent(
            float(target["small_write_median_ms"]),
            float(baseline["small_write_median_ms"]),
        )
        p95_improvement = improvement_percent(
            float(target["small_write_p95_ms"]),
            float(baseline["small_write_p95_ms"]),
        )
        stability_cv = float(target["stability_cv_percent"])
        rank = int(target["rank"]) if target.get("rank") is not None else None

        checks = {
            "rank_is_1": rank == 1,
            "throughput_gain_meets_threshold": (
                throughput_gain >= args.minimum_throughput_gain_percent
            ),
            "median_improvement_meets_threshold": (
                median_improvement >= args.minimum_latency_improvement_percent
            ),
            "p95_improvement_meets_threshold": (
                p95_improvement >= args.minimum_latency_improvement_percent
            ),
            "stability_meets_threshold": (
                stability_cv <= args.maximum_stability_cv_percent
            ),
        }
        metrics = {
            "target_rank": rank,
            "throughput_gain_vs_8_0_percent": round(throughput_gain, 2),
            "small_median_improvement_vs_8_0_percent": round(
                median_improvement, 2
            ),
            "small_p95_improvement_vs_8_0_percent": round(
                p95_improvement, 2
            ),
            "target_stability_cv_percent": round(stability_cv, 2),
        }

        if all(checks.values()):
            verdict = "confirmed"
        elif checks["rank_is_1"]:
            verdict = "supported_but_thresholds_not_all_met"
        else:
            verdict = "not_confirmed"

    fio_logs = sorted(
        str(path)
        for path in fio_dir.glob("*.log")
        if path.is_file()
    )
    expected_fio_logs = args.fio_repeat * 2
    if len(fio_logs) != expected_fio_logs:
        errors.append(
            f"fio/strace log count mismatch: "
            f"expected={expected_fio_logs} actual={len(fio_logs)}"
        )
        verdict = "invalid"

    result = {
        "schema_version": 1,
        "target": {"event_threads": 8, "write_limit": 4},
        "baseline": {"event_threads": 8, "write_limit": 0},
        "expected_runs_per_candidate": args.expected_runs,
        "fio_repeat": args.fio_repeat,
        "thresholds": {
            "minimum_throughput_gain_percent": (
                args.minimum_throughput_gain_percent
            ),
            "minimum_latency_improvement_percent": (
                args.minimum_latency_improvement_percent
            ),
            "maximum_stability_cv_percent": (
                args.maximum_stability_cv_percent
            ),
        },
        "checks": checks,
        "metrics": metrics,
        "verdict": verdict,
        "fio_logs": fio_logs,
        "errors": errors,
        "candidates": [
            cells[pair] for pair in REQUIRED_CANDIDATES if pair in cells
        ],
    }

    output_json.parent.mkdir(parents=True, exist_ok=True)
    output_md.parent.mkdir(parents=True, exist_ok=True)
    output_json.write_text(
        json.dumps(result, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )

    lines = [
        "# FOD 3.2.60 admission confirmation",
        "",
        f"Verdict: **{verdict}**",
        "",
        "Target: `FOD_FUSE_EVENT_THREADS=8`, "
        "`FOD_TASK_WRITE_ACTIVE_LIMIT=4`.",
        "",
        "Baseline: `FOD_FUSE_EVENT_THREADS=8`, "
        "`FOD_TASK_WRITE_ACTIVE_LIMIT=0`.",
        "",
    ]

    if metrics:
        lines.extend(
            [
                "## Target confirmation metrics",
                "",
                f"- target rank: {metrics['target_rank']}",
                "- throughput gain vs 8/0: "
                f"{metrics['throughput_gain_vs_8_0_percent']:.2f}%",
                "- small-write median improvement vs 8/0: "
                f"{metrics['small_median_improvement_vs_8_0_percent']:.2f}%",
                "- small-write p95 improvement vs 8/0: "
                f"{metrics['small_p95_improvement_vs_8_0_percent']:.2f}%",
                "- target stability CV: "
                f"{metrics['target_stability_cv_percent']:.2f}%",
                "",
            ]
        )

    lines.extend(
        [
            "## Candidate summary",
            "",
            "| threads | limit | rank | score | throughput MiB/s | "
            "small median ms | small p95 ms | stability CV % | overlap % |",
            "| ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |",
        ]
    )
    for pair in REQUIRED_CANDIDATES:
        cell = cells.get(pair)
        if cell is None:
            continue
        lines.append(
            f"| {pair[0]} | {pair[1]} | {cell.get('rank')} | "
            f"{cell.get('diagnostic_score')} | "
            f"{float(cell['throughput_median_mib_s']):.3f} | "
            f"{float(cell['small_write_median_ms']):.3f} | "
            f"{float(cell['small_write_p95_ms']):.3f} | "
            f"{float(cell['stability_cv_percent']):.2f} | "
            f"{float(cell['minimum_write_overlap_percent']):.2f} |"
        )

    lines.extend(
        [
            "",
            "## Fio/strace",
            "",
            f"Captured fio/strace logs: {len(fio_logs)} "
            f"(expected {expected_fio_logs}).",
            "",
            "Performance verdicts are recorded rather than used as a default "
            "commit gate. Correctness/data-completeness errors still fail.",
        ]
    )

    if errors:
        lines.extend(["", "## Errors", ""])
        lines.extend(f"- {message}" for message in errors)

    output_md.write_text("\n".join(lines) + "\n", encoding="utf-8")
    print(output_md.read_text(encoding="utf-8"), end="")

    if errors:
        return 1
    if args.require_confirmed and verdict != "confirmed":
        return 2
    return 0


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser()
    parser.add_argument("--matrix-summary", required=True)
    parser.add_argument("--output-json", required=True)
    parser.add_argument("--output-md", required=True)
    parser.add_argument("--fio-artifact-dir", required=True)
    parser.add_argument("--expected-runs", type=int, default=10)
    parser.add_argument("--fio-repeat", type=int, default=3)
    parser.add_argument(
        "--minimum-throughput-gain-percent", type=float, default=10.0
    )
    parser.add_argument(
        "--minimum-latency-improvement-percent", type=float, default=10.0
    )
    parser.add_argument(
        "--maximum-stability-cv-percent", type=float, default=25.0
    )
    parser.add_argument("--require-confirmed", action="store_true")
    return parser


if __name__ == "__main__":
    raise SystemExit(report(build_parser().parse_args()))
