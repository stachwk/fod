#!/usr/bin/env python3
"""Summarize repeated Storage Engine v2 extent-size profile artifacts."""

from __future__ import annotations

import argparse
from collections import defaultdict
from dataclasses import dataclass
from pathlib import Path
import re
from statistics import mean, pstdev


KEY_VALUE = re.compile(r"^([A-Za-z0-9_]+)=(.*)$")
RUN_SUFFIX = re.compile(r"^(local|qnap)-repeat-([0-9]+)-(block|extent-([0-9]+))$")
WORKLOAD_BEGIN = re.compile(r"^WORKLOAD_BEGIN name=([^ ]+) mode=([^ ]+) target_bytes=([0-9]+)$")
OK_THROUGHPUT = re.compile(
    r"^OK (?:large-file-multiblock|large-copy-benchmark) .*elapsed_s=([0-9.]+) throughput_mib_s=([0-9.]+)$"
)
OK_REMOUNT = re.compile(r"^OK remount-durability bytes=[0-9]+ elapsed_s=([0-9.]+) ")
FIO_READ = re.compile(r"^\s*READ: bw=([^,]+)")
FIO_WRITE = re.compile(r"^\s*WRITE: bw=([^,]+)")
PROFILE_METRIC = re.compile(r"INFO -\s+([a-z0-9_]+)=([0-9]+)$")
MAX_RSS = re.compile(r"^\s*Maximum resident set size \(kbytes\): ([0-9]+)$")
FIO_BW_VALUE = re.compile(r"^([0-9.]+)(KiB|MiB|GiB)/s")


@dataclass
class WorkloadResult:
    name: str
    elapsed_s: str = ""
    throughput_mib_s: str = ""
    read_bw: str = ""
    write_bw: str = ""
    peak_payload_bytes: int = 0
    prepare_extent_us: int = 0
    repo_persist_extents_us: int = 0
    max_rss_kib: int = 0


@dataclass
class RunResult:
    backend: str
    repeat: int
    mode: str
    target_bytes: int
    workloads: list[WorkloadResult]
    dml: dict[str, str]
    wal: dict[str, str]


def parse_key_values(path: Path) -> dict[str, str]:
    values: dict[str, str] = {}
    if not path.exists():
        return values
    for line in path.read_text(encoding="utf-8", errors="replace").splitlines():
        match = KEY_VALUE.match(line.strip())
        if match:
            values[match.group(1)] = match.group(2)
    return values


def parse_workloads(path: Path) -> list[WorkloadResult]:
    results: list[WorkloadResult] = []
    current: WorkloadResult | None = None

    for line in path.read_text(encoding="utf-8", errors="replace").splitlines():
        begin = WORKLOAD_BEGIN.match(line)
        if begin:
            current = WorkloadResult(name=begin.group(1))
            results.append(current)
            continue
        if current is None:
            continue

        throughput = OK_THROUGHPUT.match(line)
        if throughput:
            current.elapsed_s = throughput.group(1)
            current.throughput_mib_s = throughput.group(2)
            continue
        remount = OK_REMOUNT.match(line)
        if remount:
            current.elapsed_s = remount.group(1)
            continue
        read_bw = FIO_READ.match(line)
        if read_bw:
            current.read_bw = read_bw.group(1)
            continue
        write_bw = FIO_WRITE.match(line)
        if write_bw:
            current.write_bw = write_bw.group(1)
            continue
        metric = PROFILE_METRIC.search(line)
        if metric:
            key = metric.group(1)
            value = int(metric.group(2))
            if key == "prepare_persist_extent_rows_peak_payload_bytes":
                current.peak_payload_bytes = max(current.peak_payload_bytes, value)
            elif key == "prepare_persist_extent_rows_from_extent_ranges_us":
                current.prepare_extent_us += value
            elif key == "repo_persist_extents_us":
                current.repo_persist_extents_us += value
            continue
        max_rss = MAX_RSS.match(line)
        if max_rss:
            current.max_rss_kib = max(current.max_rss_kib, int(max_rss.group(1)))

    return results


def load_runs(artifact_root: Path, run_prefix: str) -> list[RunResult]:
    runs: list[RunResult] = []
    marker = f"-{run_prefix}-"
    for directory in sorted(artifact_root.iterdir() if artifact_root.exists() else []):
        if not directory.is_dir() or marker not in directory.name:
            continue
        suffix = directory.name.split(marker, 1)[1]
        match = RUN_SUFFIX.match(suffix)
        if not match:
            continue
        mode_label = match.group(3)
        mode = "block" if mode_label == "block" else "extent"
        target_bytes = 0 if mode == "block" else int(match.group(4))
        workload_log = directory / "storage-extent-workloads.log"
        if not workload_log.exists():
            continue
        runs.append(
            RunResult(
                backend=match.group(1),
                repeat=int(match.group(2)),
                mode=mode,
                target_bytes=target_bytes,
                workloads=parse_workloads(workload_log),
                dml=parse_key_values(directory / "pg_table_dml_delta-before-to-after.txt"),
                wal=parse_key_values(directory / "pg_wal_delta-before-to-after.tsv"),
            )
        )
    runs.sort(
        key=lambda run: (
            run.backend,
            run.repeat,
            0 if run.mode == "block" else 1,
            run.target_bytes,
        )
    )
    return runs


def value(values: dict[str, str], key: str) -> str:
    return values.get(key, "")


def numeric(value: str) -> float | None:
    try:
        return float(value)
    except ValueError:
        return None


def bandwidth_kib_s(value: str) -> float | None:
    match = FIO_BW_VALUE.match(value)
    if not match:
        return None
    multiplier = {"KiB": 1.0, "MiB": 1024.0, "GiB": 1024.0 * 1024.0}[match.group(2)]
    return float(match.group(1)) * multiplier


def format_mean(values: list[float], digits: int = 2) -> str:
    if not values:
        return ""
    return f"{mean(values):.{digits}f}"


def render(runs: list[RunResult], run_prefix: str) -> str:
    lines = [
        f"# Storage extent matrix: {run_prefix}",
        "",
        "## Aggregate",
        "",
        "| backend | mode | target bytes | workload | samples | throughput mean MiB/s | throughput stdev | throughput min | throughput max | read mean KiB/s | write mean KiB/s | elapsed mean s | peak payload bytes | max RSS mean KiB | run block inserts mean | run extent inserts mean | run WAL bytes mean |",
        "| --- | --- | ---: | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |",
    ]
    grouped: dict[tuple[str, str, int, str], list[tuple[RunResult, WorkloadResult]]] = (
        defaultdict(list)
    )
    for run in runs:
        for workload in run.workloads:
            grouped[(run.backend, run.mode, run.target_bytes, workload.name)].append(
                (run, workload)
            )

    for key in sorted(
        grouped,
        key=lambda item: (item[0], 0 if item[1] == "block" else 1, item[2], item[3]),
    ):
        samples = grouped[key]
        throughputs = [
            parsed
            for _, workload in samples
            if (parsed := numeric(workload.throughput_mib_s)) is not None
        ]
        elapsed = [
            parsed
            for _, workload in samples
            if (parsed := numeric(workload.elapsed_s)) is not None
        ]
        read_bw = [
            parsed
            for _, workload in samples
            if (parsed := bandwidth_kib_s(workload.read_bw)) is not None
        ]
        write_bw = [
            parsed
            for _, workload in samples
            if (parsed := bandwidth_kib_s(workload.write_bw)) is not None
        ]
        rss = [float(workload.max_rss_kib) for _, workload in samples]
        block_inserts = [
            parsed
            for run, _ in samples
            if (parsed := numeric(value(run.dml, "data_blocks_n_tup_ins_delta")))
            is not None
        ]
        extent_inserts = [
            parsed
            for run, _ in samples
            if (parsed := numeric(value(run.dml, "data_extents_n_tup_ins_delta")))
            is not None
        ]
        wal_bytes = [
            parsed
            for run, _ in samples
            if (parsed := numeric(value(run.wal, "wal_bytes_delta"))) is not None
        ]
        peak_payload = max(workload.peak_payload_bytes for _, workload in samples)
        lines.append(
            "| "
            + " | ".join(
                [
                    key[0],
                    key[1],
                    str(key[2]),
                    key[3],
                    str(len(samples)),
                    format_mean(throughputs),
                    f"{pstdev(throughputs):.2f}" if throughputs else "",
                    f"{min(throughputs):.2f}" if throughputs else "",
                    f"{max(throughputs):.2f}" if throughputs else "",
                    format_mean(read_bw),
                    format_mean(write_bw),
                    format_mean(elapsed, 6),
                    str(peak_payload),
                    format_mean(rss),
                    format_mean(block_inserts),
                    format_mean(extent_inserts),
                    format_mean(wal_bytes),
                ]
            )
            + " |"
        )

    lines.extend(
        [
            "",
            "## Raw runs",
            "",
            "| backend | repeat | mode | target bytes | workload | elapsed s | throughput MiB/s | read bw | write bw | peak payload bytes | prepare extent us | persist extents us | max RSS KiB | run block inserts | run extent inserts | run WAL bytes |",
        "| --- | ---: | --- | ---: | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |",
        ]
    )
    for run in runs:
        for workload in run.workloads:
            lines.append(
                "| "
                + " | ".join(
                    [
                        run.backend,
                        str(run.repeat),
                        run.mode,
                        str(run.target_bytes),
                        workload.name,
                        workload.elapsed_s,
                        workload.throughput_mib_s,
                        workload.read_bw,
                        workload.write_bw,
                        str(workload.peak_payload_bytes),
                        str(workload.prepare_extent_us),
                        str(workload.repo_persist_extents_us),
                        str(workload.max_rss_kib),
                        value(run.dml, "data_blocks_n_tup_ins_delta"),
                        value(run.dml, "data_extents_n_tup_ins_delta"),
                        value(run.wal, "wal_bytes_delta"),
                    ]
                )
                + " |"
            )
    lines.append("")
    return "\n".join(lines)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--artifact-root", type=Path, required=True)
    parser.add_argument("--run-prefix", required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()

    runs = load_runs(args.artifact_root, args.run_prefix)
    if not runs:
        parser.error(f"no storage extent artifacts found for run prefix {args.run_prefix!r}")
    output = render(runs, args.run_prefix)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(output, encoding="utf-8")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
