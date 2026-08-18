#!/usr/bin/env python
# -*- coding: utf-8 -*-

# Stala macierz regresji sekwencyjnego odczytu FOD.
# Mierzy tylko produkcyjny block path i rozdziela oba tryby direct-I/O.

import argparse
import os
import re
import shutil
import statistics
import subprocess
import sys
from datetime import datetime, timezone
from pathlib import Path


READ_RE = re.compile(
    r"(?m)^\s*READ:\s+bw=([0-9]+(?:\.[0-9]+)?)([KMG]?i?B)/s"
    r".*?\brun=([0-9]+)(?:-([0-9]+))?msec"
)
WRITE_RE = re.compile(
    r"(?m)^\s*WRITE:\s+bw=([0-9]+(?:\.[0-9]+)?)([KMG]?i?B)/s"
    r".*?\brun=([0-9]+)(?:-([0-9]+))?msec"
)


def print_err(str):
    print(f"stderr: {str}", file=sys.stderr)


def run_cmd(cmd, cwd, env=None, check=True):
    p = subprocess.run(
        cmd,
        cwd=str(cwd),
        env=env,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
    )
    if check and p.returncode != 0:
        raise RuntimeError(
            f"command failed: {' '.join(cmd)}\nexit={p.returncode}\n{p.stdout}"
        )
    return p


def to_mib_s(value, unit):
    factors = {
        "B": 1.0 / (1024.0 * 1024.0),
        "KB": 1000.0 / (1024.0 * 1024.0),
        "MB": 1000.0 * 1000.0 / (1024.0 * 1024.0),
        "GB": 1000.0 * 1000.0 * 1000.0 / (1024.0 * 1024.0),
        "KiB": 1.0 / 1024.0,
        "MiB": 1.0,
        "GiB": 1024.0,
    }
    if unit not in factors:
        raise RuntimeError(f"unsupported fio bandwidth unit: {unit}")
    return float(value) * factors[unit]


def parse_exact_metric(output, regex, label):
    matches = list(regex.finditer(output))
    if len(matches) != 1:
        raise RuntimeError(
            f"expected exactly one fio {label} summary for the block-only benchmark, "
            f"found {len(matches)}; refusing ambiguous benchmark"
        )
    m = matches[0]
    return to_mib_s(m.group(1), m.group(2)), int(m.group(4) or m.group(3))


def read_sysfs_text(path):
    try:
        return Path(path).read_text(encoding="utf-8").strip()
    except (OSError, UnicodeError):
        return None


def collect_power_metadata():
    power_online = []
    battery_state = []
    root = Path("/sys/class/power_supply")
    if root.is_dir():
        for supply in sorted(root.iterdir()):
            online = read_sysfs_text(supply / "online")
            if online is not None:
                power_online.append(f"{supply.name}={online}")
            status = read_sysfs_text(supply / "status")
            capacity = read_sysfs_text(supply / "capacity")
            if status is not None or capacity is not None:
                parts = [supply.name]
                if status is not None:
                    parts.append(f"status={status}")
                if capacity is not None:
                    parts.append(f"capacity={capacity}%")
                battery_state.append(" ".join(parts))

    cpu0 = Path("/sys/devices/system/cpu/cpu0/cpufreq")
    governors = set()
    epp_values = set()
    cpu_root = Path("/sys/devices/system/cpu")
    for path in cpu_root.glob("cpu*/cpufreq/scaling_governor"):
        value = read_sysfs_text(path)
        if value:
            governors.add(value)
    for path in cpu_root.glob("cpu*/cpufreq/energy_performance_preference"):
        value = read_sysfs_text(path)
        if value:
            epp_values.add(value)

    return {
        "power_online": ", ".join(power_online) if power_online else "unknown",
        "battery_state": "; ".join(battery_state) if battery_state else "unknown",
        "governors": ", ".join(sorted(governors)) if governors else "unknown",
        "scaling_driver": read_sysfs_text(cpu0 / "scaling_driver") or "unknown",
        "scaling_min_freq_khz": read_sysfs_text(cpu0 / "scaling_min_freq") or "unknown",
        "scaling_max_freq_khz": read_sysfs_text(cpu0 / "scaling_max_freq") or "unknown",
        "epp": ", ".join(sorted(epp_values)) if epp_values else "unknown",
    }

def summarize(rows):
    reads = [r["read_mib_s"] for r in rows]
    writes = [r["write_mib_s"] for r in rows]
    runtimes = [r["read_ms"] for r in rows]
    return {
        "read_mean": statistics.mean(reads),
        "read_median": statistics.median(reads),
        "read_min": min(reads),
        "read_max": max(reads),
        "read_stdev": statistics.pstdev(reads),
        "write_median": statistics.median(writes),
        "read_runtime_median_ms": statistics.median(runtimes),
    }


def pct(current, baseline):
    return ((current / baseline) - 1.0) * 100.0 if baseline else 0.0


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--repeat", type=int, default=5)
    parser.add_argument("--sizes", nargs="+", default=["4M", "128M"])
    parser.add_argument("--block-size", default="4k")
    parser.add_argument("--artifact-root", default=None)
    parser.add_argument("--record", default=None)
    parser.add_argument("--verbose", type=int, choices=(0, 1), default=0)
    args = parser.parse_args()

    if args.repeat < 3 or args.repeat > 30:
        raise RuntimeError("--repeat must be in range 3..30")

    repo = Path(__file__).resolve().parents[2]
    if not (repo / ".git").exists():
        raise RuntimeError(f"repository root not found: {repo}")
    if not shutil.which("fio") or not shutil.which("make"):
        raise RuntimeError("fio and make are required")

    head = run_cmd(["git", "rev-parse", "HEAD"], repo).stdout.strip()
    version = (repo / "fod_version.txt").read_text(encoding="utf-8").strip()
    host = run_cmd(["hostname", "-s"], repo, check=False).stdout.strip() or "unknown-host"
    stamp = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ")
    artifact_root = (
        Path(args.artifact_root).expanduser().resolve()
        if args.artifact_root
        else repo / "artifacts" / "perf" / head[:7]
    )
    artifact_dir = artifact_root / f"{host}-block-read-matrix-{version}-{stamp}"
    artifact_dir.mkdir(parents=True, exist_ok=False)

    power = collect_power_metadata()

    if args.verbose:
        print_err(f"repo: {repo}")
        print_err(f"commit: {head}")
        print_err(f"version: {version}")
        print_err(f"artifact_dir: {artifact_dir}")
        print_err(f"power_online: {power['power_online']}")
        print_err(f"battery_state: {power['battery_state']}")
        print_err(f"cpu_governors: {power['governors']}")
        print_err(f"cpu_scaling_driver: {power['scaling_driver']}")
        print_err(f"cpu_scaling_min_freq_khz: {power['scaling_min_freq_khz']}")
        print_err(f"cpu_scaling_max_freq_khz: {power['scaling_max_freq_khz']}")
        print_err(f"cpu_energy_performance_preference: {power['epp']}")

    run_cmd(["make", "--no-print-directory", "build-debug"], repo)

    rows = []
    for direct_io in (0, 1):
        for size in args.sizes:
            for run_no in range(1, args.repeat + 1):
                env = os.environ.copy()
                env["FOD_PROFILE_IO"] = "1"
                env["FOD_FOPEN_DIRECT_IO"] = str(direct_io)
                env["FIO_FILE_SIZE"] = size
                env["FIO_BLOCK_SIZE"] = args.block_size

                if args.verbose:
                    print_err(
                        f"run direct_io={direct_io} size={size} {run_no}/{args.repeat}"
                    )

                p = run_cmd(
                    ["make", "--no-print-directory", "test-fio-sequential-io"],
                    repo,
                    env=env,
                    check=False,
                )
                log_path = artifact_dir / (
                    f"direct-{direct_io}-size-{size}-run-{run_no:02d}.log"
                )
                log_path.write_text(p.stdout, encoding="utf-8")
                if p.returncode != 0:
                    raise RuntimeError(
                        f"benchmark failed for direct_io={direct_io} size={size}; "
                        f"log={log_path}"
                    )

                read_mib_s, read_ms = parse_exact_metric(p.stdout, READ_RE, "READ")
                write_mib_s, write_ms = parse_exact_metric(p.stdout, WRITE_RE, "WRITE")
                rows.append(
                    {
                        "direct_io": direct_io,
                        "size": size,
                        "run": run_no,
                        "read_mib_s": read_mib_s,
                        "read_ms": read_ms,
                        "write_mib_s": write_mib_s,
                        "write_ms": write_ms,
                    }
                )
                if args.verbose:
                    print_err(
                        f"result read={read_mib_s:.3f} MiB/s "
                        f"write={write_mib_s:.3f} MiB/s"
                    )

    groups = {}
    for direct_io in (0, 1):
        for size in args.sizes:
            groups[(direct_io, size)] = summarize(
                [r for r in rows if r["direct_io"] == direct_io and r["size"] == size]
            )

    now = datetime.now(timezone.utc)
    title = f"## {now.strftime('%Y-%m-%d')} FOD {version} block read direct-I/O matrix"
    lines = [
        title,
        "",
        f"Measured commit: `{head}`. Host: `{host}`. "
        f"Collection time: `{now.strftime('%Y-%m-%d %H:%M:%S UTC')}`.",
        "",
        "Environment:",
        "",
        f"- power online: `{power['power_online']}`",
        f"- battery state: `{power['battery_state']}`",
        f"- CPU governors: `{power['governors']}`",
        f"- CPU scaling driver: `{power['scaling_driver']}`",
        f"- CPU scaling min/max: `{power['scaling_min_freq_khz']}..{power['scaling_max_freq_khz']} kHz`",
        f"- CPU energy-performance preference: `{power['epp']}`",
        "",
        "Method: production block-storage path only; "
        "`test_fio_sequential_io.sh` is block-only, fio sync engine uses `direct=0`, explicit "
        f"`FIO_BLOCK_SIZE={args.block_size}`, sizes "
        + ", ".join(f"`{size}`" for size in args.sizes)
        + f", `{args.repeat}` repetitions per cell. "
        "`FOD_FOPEN_DIRECT_IO=0` and `1` are separate baselines.",
        "",
        f"Raw logs: `{artifact_dir.relative_to(repo)}`.",
        "",
        "| file size | FOD_FOPEN_DIRECT_IO | read mean MiB/s | read median MiB/s | "
        "min-max MiB/s | stdev | median read ms | median write MiB/s |",
        "| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |",
    ]

    for size in args.sizes:
        for direct_io in (0, 1):
            s = groups[(direct_io, size)]
            lines.append(
                f"| `{size}` | `{direct_io}` | `{s['read_mean']:.3f}` | "
                f"`{s['read_median']:.3f}` | `{s['read_min']:.3f}-{s['read_max']:.3f}` | "
                f"`{s['read_stdev']:.3f}` | `{s['read_runtime_median_ms']:.0f}` | "
                f"`{s['write_median']:.3f}` |"
            )

    lines += ["", "Direct-I/O effect within the same size:", ""]
    for size in args.sizes:
        normal = groups[(0, size)]["read_median"]
        direct = groups[(1, size)]["read_median"]
        lines.append(
            f"- `{size}`: `{pct(direct, normal):+.2f}%` "
            f"(`{normal:.3f}` -> `{direct:.3f} MiB/s`)."
        )

    lines += [
        "",
        "Per-run values:",
        "",
        "| size | direct_io | run | read MiB/s | read ms | write MiB/s |",
        "| --- | ---: | ---: | ---: | ---: | ---: |",
    ]
    for r in rows:
        lines.append(
            f"| `{r['size']}` | `{r['direct_io']}` | `{r['run']}` | "
            f"`{r['read_mib_s']:.3f}` | `{r['read_ms']}` | `{r['write_mib_s']:.3f}` |"
        )

    lines += [
        "",
        "Comparison rule: compare only the same storage path, file size, fio block size, "
        "direct-I/O mode, host/backend class, power source, CPU power policy, and comparable runtime configuration. "
        "Legacy pre-retirement collectors that mixed storage modes are not block-path baselines.",
        "",
    ]

    summary = "\n".join(lines)
    summary_path = artifact_dir / "block-read-matrix-summary.md"
    summary_path.write_text(summary, encoding="utf-8")

    if args.record:
        path = Path(args.record)
        if not path.is_absolute():
            path = repo / path
        text = path.read_text(encoding="utf-8")
        if title in text:
            raise RuntimeError(f"BENCHMARKS.md already contains: {title}")
        markers = [
            "## 2026-08-17 FOD 3.2.82 pre-commit read regression check",
            "## 2026-08-17 FOD 3.2.81 recent read-path regression check",
            "## 2026-07-12 fuser 0.17 Migration Baseline",
        ]
        marker = next((m for m in markers if m in text), None)
        if marker is None:
            raise RuntimeError("no BENCHMARKS.md insertion marker found")
        path.write_text(
            text.replace(marker, summary.rstrip() + "\n\n" + marker, 1),
            encoding="utf-8",
        )
        check = run_cmd(["git", "diff", "--check"], repo, check=False)
        if check.returncode != 0:
            raise RuntimeError("git diff --check failed after BENCHMARKS.md update")

    print("OK: FOD block read regression matrix completed")
    print(f"commit={head}")
    print(f"version={version}")
    print(f"summary={summary_path}")
    for size in args.sizes:
        for direct_io in (0, 1):
            s = groups[(direct_io, size)]
            print(
                f"size={size} direct_io={direct_io} "
                f"read_mean={s['read_mean']:.3f} "
                f"read_median={s['read_median']:.3f} MiB/s"
            )
    if args.record:
        print(f"recorded={args.record}")


if __name__ == "__main__":
    try:
        main()
    except Exception as exc:
        print_err(str(exc))
        sys.exit(1)
