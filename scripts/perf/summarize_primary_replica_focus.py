#!/usr/bin/env python3
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1

from __future__ import annotations

import argparse
import csv
import statistics
from collections import defaultdict
from pathlib import Path


METRICS = (
    "primary_write_mib_s",
    "primary_write_iops",
    "primary_read_mib_s",
    "primary_read_iops",
    "replica_read_mib_s",
    "replica_read_iops",
)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Aggregate repeated FOD primary/replica matrix summaries."
    )
    parser.add_argument("summaries", nargs="+", type=Path)
    parser.add_argument("--output-tsv", required=True, type=Path)
    parser.add_argument("--output-markdown", required=True, type=Path)
    return parser.parse_args()


def load_rows(paths: list[Path]) -> dict[tuple[str, str], list[dict[str, str]]]:
    grouped: dict[tuple[str, str], list[dict[str, str]]] = defaultdict(list)
    for path in paths:
        with path.open(newline="", encoding="utf-8") as handle:
            reader = csv.DictReader(handle, delimiter="\t")
            required = {"block_size", "file_size", *METRICS}
            missing = required.difference(reader.fieldnames or ())
            if missing:
                raise SystemExit(f"{path}: missing columns: {', '.join(sorted(missing))}")
            for row in reader:
                grouped[(row["block_size"], row["file_size"])].append(row)
    return grouped


def stats(values: list[float]) -> tuple[float, float, float, float, float]:
    mean = statistics.fmean(values)
    median = statistics.median(values)
    minimum = min(values)
    maximum = max(values)
    stdev = statistics.stdev(values) if len(values) > 1 else 0.0
    return mean, median, minimum, maximum, stdev


def size_bytes(value: str) -> int:
    text = value.strip().lower()
    multiplier = 1
    for suffix, factor in (
        ("k", 1024),
        ("m", 1024**2),
        ("g", 1024**3),
        ("t", 1024**4),
    ):
        if text.endswith(suffix):
            multiplier = factor
            text = text[:-1]
            break
    return int(text) * multiplier


def aggregate(
    grouped: dict[tuple[str, str], list[dict[str, str]]],
) -> list[dict[str, str]]:
    result: list[dict[str, str]] = []
    ordered = sorted(
        grouped.items(),
        key=lambda item: (size_bytes(item[0][1]), size_bytes(item[0][0])),
    )
    for (block_size, file_size), rows in ordered:
        out: dict[str, str] = {
            "block_size": block_size,
            "file_size": file_size,
            "runs": str(len(rows)),
        }
        means: dict[str, float] = {}
        for metric in METRICS:
            values = [float(row[metric]) for row in rows]
            mean, median, minimum, maximum, stdev = stats(values)
            means[metric] = mean
            out[f"{metric}_mean"] = f"{mean:.3f}"
            out[f"{metric}_median"] = f"{median:.3f}"
            out[f"{metric}_min"] = f"{minimum:.3f}"
            out[f"{metric}_max"] = f"{maximum:.3f}"
            out[f"{metric}_stdev"] = f"{stdev:.3f}"

        primary_read = means["primary_read_mib_s"]
        replica_read = means["replica_read_mib_s"]
        out["replica_vs_primary_read_pct"] = (
            f"{((replica_read / primary_read) - 1.0) * 100.0:.2f}"
            if primary_read
            else "nan"
        )
        result.append(out)
    return result


def write_tsv(path: Path, rows: list[dict[str, str]]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    fieldnames = list(rows[0]) if rows else ["block_size", "file_size", "runs"]
    with path.open("w", newline="", encoding="utf-8") as handle:
        writer = csv.DictWriter(handle, delimiter="\t", fieldnames=fieldnames)
        writer.writeheader()
        writer.writerows(rows)


def write_markdown(path: Path, rows: list[dict[str, str]]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("w", encoding="utf-8") as handle:
        handle.write("# Primary/replica focus summary\n\n")
        handle.write(
            "| block | file | runs | primary write MiB/s | primary read MiB/s | "
            "replica read MiB/s | replica vs primary read |\n"
        )
        handle.write("| --- | --- | ---: | ---: | ---: | ---: | ---: |\n")
        for row in rows:
            handle.write(
                f"| `{row['block_size']}` | `{row['file_size']}` | {row['runs']} | "
                f"{row['primary_write_mib_s_mean']} ± {row['primary_write_mib_s_stdev']} | "
                f"{row['primary_read_mib_s_mean']} ± {row['primary_read_mib_s_stdev']} | "
                f"{row['replica_read_mib_s_mean']} ± {row['replica_read_mib_s_stdev']} | "
                f"{row['replica_vs_primary_read_pct']}% |\n"
            )


def main() -> int:
    args = parse_args()
    grouped = load_rows(args.summaries)
    rows = aggregate(grouped)
    if not rows:
        raise SystemExit("No benchmark rows found.")
    write_tsv(args.output_tsv, rows)
    write_markdown(args.output_markdown, rows)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
