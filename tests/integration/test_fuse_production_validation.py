#!/usr/bin/env python3
from __future__ import annotations
import argparse
import json
import os
import re
import statistics
import subprocess
import time
from pathlib import Path

OBS_RE = re.compile(
    r"FOD logical task observability:.*?stage=shutdown lane=(?P<lane>read|write).*?"
    r"peak_queued_tasks=(?P<peak_queued>[0-9]+).*?"
    r"peak_active_tasks=(?P<peak_active>[0-9]+).*?"
    r"accounting_errors=(?P<accounting_errors>[0-9]+)"
)

CASES = [
    ("sequential", ["make", "--no-print-directory", "test-fio-sequential-io", "FIO_CASES=block", "FIO_FILE_SIZE=4M"]),
    ("mixed", ["make", "--no-print-directory", "test-fio-mixed-io", "FIO_CASES=block", "FIO_FILE_SIZE=8M"]),
    ("random-mixed", ["make", "--no-print-directory", "test-fio-random-mixed-io", "FIO_CASES=block", "FIO_FILE_SIZE=8M"]),
]
CONFIGS = {
    "target-8-4": {
        "FOD_FUSE_EVENT_THREADS": "8", "FOD_FUSE_CLONE_FD": "0",
        "FOD_TASK_READ_ACTIVE_LIMIT": "0", "FOD_TASK_WRITE_ACTIVE_LIMIT": "4",
        "FOD_PROFILE_IO": "1",
    },
    "baseline-8-0": {
        "FOD_FUSE_EVENT_THREADS": "8", "FOD_FUSE_CLONE_FD": "0",
        "FOD_TASK_READ_ACTIVE_LIMIT": "0", "FOD_TASK_WRITE_ACTIVE_LIMIT": "0",
        "FOD_PROFILE_IO": "1",
    },
}
PG_FIELDS = [
    "xact_commit", "xact_rollback", "blks_read", "blks_hit", "temp_files",
    "temp_bytes", "blk_read_time", "blk_write_time", "session_time", "active_time",
]

def run_logged(root, command, env_extra, log_path):
    env = os.environ.copy(); env.update(env_extra)
    started = time.monotonic()
    cp = subprocess.run(command, cwd=root, env=env, text=True,
                        stdout=subprocess.PIPE, stderr=subprocess.STDOUT, check=False)
    elapsed = time.monotonic() - started
    log_path.write_text(cp.stdout, encoding="utf-8")
    print(cp.stdout, end="")
    if cp.returncode != 0:
        raise RuntimeError(f"rc={cp.returncode}: {' '.join(command)}; log={log_path}")
    return elapsed, cp.stdout

def parse_obs(text):
    result = {}
    for m in OBS_RE.finditer(text):
        result[m.group("lane")] = {
            "peak_queued": int(m.group("peak_queued")),
            "peak_active": int(m.group("peak_active")),
            "accounting_errors": int(m.group("accounting_errors")),
        }
    return result

def validate_obs(config_name, case_name, run_index, obs, errors):
    for lane, metric in obs.items():
        if metric["accounting_errors"]:
            errors.append(
                f"{config_name}/{case_name}/run-{run_index}: "
                f"{lane} accounting_errors={metric['accounting_errors']}"
            )
        if config_name == "target-8-4" and lane == "write" and metric["peak_active"] > 4:
            errors.append(
                f"{config_name}/{case_name}/run-{run_index}: "
                f"write peak_active={metric['peak_active']} > 4"
            )

def pg_snapshot(root):
    user = os.environ.get("POSTGRES_USER", "foduser")
    database = os.environ.get("POSTGRES_DB", "foddbname")
    password = os.environ.get("POSTGRES_PASSWORD", "")
    host = os.environ.get("FOD_PG_HOST", "127.0.0.1")
    port = os.environ.get("POSTGRES_PORT") or os.environ.get("FOD_PG_PORT") or "5432"
    sql = "SELECT " + ",".join(PG_FIELDS) + " FROM pg_stat_database WHERE datname=current_database();"
    env = os.environ.copy(); env["PGPASSWORD"] = password
    cp = subprocess.run(
        ["psql", "-X", "-A", "-t", "-F", "|", "-v", "ON_ERROR_STOP=1",
         "-h", host, "-p", str(port), "-U", user, "-d", database, "-c", sql],
        cwd=root, env=env, text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
        check=False,
    )
    if cp.returncode != 0:
        return {"available": False, "error": cp.stderr.strip()}
    line = cp.stdout.strip().splitlines()[-1] if cp.stdout.strip() else ""
    values = line.split("|")
    if len(values) != len(PG_FIELDS):
        return {"available": False, "error": f"unexpected pg_stat_database row: {line!r}"}
    result = {"available": True}
    for name, value in zip(PG_FIELDS, values):
        try:
            result[name] = float(value) if "." in value else int(value)
        except ValueError:
            result[name] = 0.0
    return result

def pg_delta(before, after):
    if not before.get("available") or not after.get("available"):
        return {"available": False, "before_error": before.get("error"), "after_error": after.get("error")}
    result = {"available": True}
    for name in PG_FIELDS:
        result[name] = round(float(after[name]) - float(before[name]), 6)
    xacts = result["xact_commit"] + result["xact_rollback"]
    result["transactions"] = int(xacts)
    result["active_time_per_transaction_ms_proxy"] = (
        round(float(result["active_time"]) / xacts, 6) if xacts > 0 else None
    )
    io_total = float(result["blks_hit"]) + float(result["blks_read"])
    result["cache_hit_ratio_percent"] = (
        round(float(result["blks_hit"]) * 100.0 / io_total, 4) if io_total > 0 else None
    )
    return result

def run_endurance(root, artifact_dir, config_name, seconds, errors):
    started = time.monotonic(); iterations = []; index = 0
    while time.monotonic() - started < seconds:
        index += 1
        log_path = artifact_dir / f"{config_name}-endurance-randrw-{index}.log"
        before = pg_snapshot(root)
        elapsed, text = run_logged(
            root,
            ["make", "--no-print-directory", "test-fio-random-mixed-io", "FIO_CASES=block", "FIO_FILE_SIZE=8M"],
            CONFIGS[config_name], log_path,
        )
        after = pg_snapshot(root)
        obs = parse_obs(text)
        validate_obs(config_name, "endurance-randrw", index, obs, errors)
        iterations.append({
            "iteration": index, "elapsed_seconds": round(elapsed, 6),
            "observability": obs, "postgres": pg_delta(before, after), "log": str(log_path),
        })
    total = time.monotonic() - started
    return {
        "requested_seconds": seconds,
        "elapsed_seconds": round(total, 6),
        "iterations": len(iterations),
        "iteration_median_seconds": round(statistics.median(x["elapsed_seconds"] for x in iterations), 6) if iterations else None,
        "runs": iterations,
    }

def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", default=".")
    parser.add_argument("--artifact-dir", required=True)
    parser.add_argument("--repeat", type=int, default=3)
    parser.add_argument("--endurance-seconds-per-config", type=int, default=150)
    args = parser.parse_args()
    if args.repeat < 1 or args.endurance_seconds_per_config < 1:
        raise SystemExit("repeat and endurance seconds must be >= 1")
    root = Path(args.root).resolve()
    artifact_dir = Path(args.artifact_dir).resolve(); artifact_dir.mkdir(parents=True, exist_ok=True)

    rows = []; correctness_errors = []; pg_warnings = []
    names = list(CONFIGS)

    # Fail fast on mount/unmount contract before spending time on repeated fio
    # and endurance work.
    mount_log = artifact_dir / "target-8-4-mount-suite.log"
    mount_elapsed, _ = run_logged(
        root,
        ["make", "--no-print-directory", "test-mount-suite"],
        CONFIGS["target-8-4"],
        mount_log,
    )

    for run_index in range(1, args.repeat + 1):
        offset = run_index % len(names); order = names[offset:] + names[:offset]
        for config_name in order:
            for case_name, command in CASES:
                log_path = artifact_dir / f"{config_name}-{case_name}-run-{run_index}.log"
                before = pg_snapshot(root)
                elapsed, text = run_logged(root, command, CONFIGS[config_name], log_path)
                after = pg_snapshot(root)
                delta = pg_delta(before, after)
                if not delta.get("available"):
                    pg_warnings.append(f"{config_name}/{case_name}/run-{run_index}: {delta}")
                obs = parse_obs(text); validate_obs(config_name, case_name, run_index, obs, correctness_errors)
                rows.append({
                    "config": config_name, "case": case_name, "run": run_index,
                    "elapsed_seconds": round(elapsed, 6), "observability": obs,
                    "postgres": delta, "log": str(log_path),
                })

    strace = []
    for config_name in names:
        log_path = artifact_dir / f"{config_name}-sequential-strace.log"
        elapsed, _ = run_logged(
            root,
            ["make", "--no-print-directory", "test-fio-sequential-io-strace", "FIO_CASES=block", "FIO_FILE_SIZE=4M"],
            CONFIGS[config_name], log_path,
        )
        strace.append({"config": config_name, "elapsed_seconds": round(elapsed, 6), "log": str(log_path)})

    endurance = {
        name: run_endurance(root, artifact_dir, name, args.endurance_seconds_per_config, correctness_errors)
        for name in names
    }

    grouped = {}
    for row in rows:
        grouped.setdefault((row["config"], row["case"]), []).append(row["elapsed_seconds"])
    summaries = []; checks = {}
    for case_name, _ in CASES:
        target = statistics.median(grouped[("target-8-4", case_name)])
        baseline = statistics.median(grouped[("baseline-8-0", case_name)])
        delta = ((target - baseline) * 100.0 / baseline) if baseline else 0.0
        checks[case_name] = delta <= 20.0
        summaries.append({
            "case": case_name, "target_median_seconds": round(target, 6),
            "baseline_median_seconds": round(baseline, 6),
            "target_vs_baseline_percent": round(delta, 2),
            "no_gt_20_percent_wall_regression": checks[case_name],
        })

    target_e = endurance["target-8-4"]; baseline_e = endurance["baseline-8-0"]
    endurance_delta = None
    if target_e["iteration_median_seconds"] is not None and baseline_e["iteration_median_seconds"]:
        endurance_delta = round(
            (target_e["iteration_median_seconds"] - baseline_e["iteration_median_seconds"])
            * 100.0 / baseline_e["iteration_median_seconds"], 2
        )

    verdict = (
        "invalid" if correctness_errors else
        "production_candidate_supported" if all(checks.values()) else
        "performance_regression_observed"
    )
    report = {
        "schema_version": 2,
        "target": {"fuse_event_threads": 8, "fuse_clone_fd": False, "task_read_active_limit": 0, "task_write_active_limit": 4},
        "baseline": {"fuse_event_threads": 8, "fuse_clone_fd": False, "task_read_active_limit": 0, "task_write_active_limit": 0},
        "repeat": args.repeat,
        "endurance_seconds_per_config": args.endurance_seconds_per_config,
        "summaries": summaries,
        "performance_checks": checks,
        "correctness_errors": correctness_errors,
        "postgres_snapshot_warnings": pg_warnings,
        "strace_logs": strace,
        "mount_suite": {"elapsed_seconds": round(mount_elapsed, 6), "log": str(mount_log)},
        "endurance": endurance,
        "endurance_target_vs_baseline_median_percent": endurance_delta,
        "runs": rows,
        "verdict": verdict,
    }
    (artifact_dir / "production-validation-summary.json").write_text(
        json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )
    lines = [
        "# FOD 3.2.61 production validation", "", f"Verdict: **{verdict}**", "",
        "| workload | 8/4 median s | 8/0 median s | 8/4 delta % | <=20% regression |",
        "| --- | ---: | ---: | ---: | :---: |",
    ]
    for row in summaries:
        lines.append(
            f"| {row['case']} | {row['target_median_seconds']:.3f} | {row['baseline_median_seconds']:.3f} | "
            f"{row['target_vs_baseline_percent']:.2f} | {'yes' if row['no_gt_20_percent_wall_regression'] else 'no'} |"
        )
    lines += [
        "", "## Endurance", "",
        f"- 8/4: {target_e['elapsed_seconds']:.1f}s, iterations={target_e['iterations']}, median={target_e['iteration_median_seconds']}s.",
        f"- 8/0: {baseline_e['elapsed_seconds']:.1f}s, iterations={baseline_e['iterations']}, median={baseline_e['iteration_median_seconds']}s.",
        f"- 8/4 median-iteration delta: {endurance_delta}%.",
        "", "## PostgreSQL", "",
        "`pg_stat_database` deltas are captured around normal and endurance workloads. "
        "`active_time_per_transaction_ms_proxy` is an active-time/xact proxy, not direct client transaction latency.",
        f"PostgreSQL snapshot warnings: {len(pg_warnings)}.",
        "", "## Additional validation", "",
        f"Target mount-suite duration: {mount_elapsed:.3f}s.",
        f"Strace-backed sequential logs: {len(strace)}.",
        "", "Correctness/accounting failures are blocking. Performance differences are recorded as evidence.",
    ]
    if correctness_errors:
        lines += ["", "## Correctness errors", ""] + [f"- {x}" for x in correctness_errors]
    md = "\n".join(lines) + "\n"
    (artifact_dir / "production-validation-summary.md").write_text(md, encoding="utf-8")
    print(md, end="")
    return 1 if correctness_errors else 0

if __name__ == "__main__":
    raise SystemExit(main())
