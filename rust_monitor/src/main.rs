// Copyright (c) 2026 Wojciech Stach
// Licensed under BSL 1.1

mod cluster;

use std::env;
use std::fs;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const FOD_VERSION: &str = env!("CARGO_PKG_VERSION");
const FOD_PROCESS_NAMES: &[&str] = &[
    "fod-rust-fuse",
    "fod-indexer",
    "fod-bootstrap",
    "fod-change",
    "mkfs.fod",
];
const DEFAULT_TOP_INTERVAL_SECONDS: u64 = 2;
const MAX_CMDLINE_CHARS: usize = 120;

#[derive(Debug, Clone, PartialEq, Eq)]
struct SystemSnapshot {
    load_average: Option<String>,
    uptime_seconds: Option<u64>,
    mem_total_bytes: Option<u64>,
    mem_available_bytes: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ProcessSnapshot {
    pid: u32,
    command: String,
    state: String,
    rss_bytes: Option<u64>,
    vm_size_bytes: Option<u64>,
    threads: Option<u64>,
    fd_count: Option<u64>,
    voluntary_context_switches: Option<u64>,
    nonvoluntary_context_switches: Option<u64>,
    cmdline: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct MonitorSnapshot {
    system: SystemSnapshot,
    processes: Vec<ProcessSnapshot>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TopOptions {
    interval: Duration,
    iterations: Option<u64>,
    clear_screen: bool,
}

fn main() {
    if let Err(err) = run() {
        eprintln!("{err}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let mut args = env::args().skip(1);
    match args.next().as_deref() {
        None | Some("status") => {
            ensure_no_extra_args(args)?;
            let cluster_snapshot = cluster::load_cluster_snapshot();
            let snapshot = monitor_snapshot()?;
            print_status(&snapshot, cluster_snapshot.as_ref().ok());
            if let Err(err) = cluster_snapshot {
                eprintln!("FOD shared cluster telemetry unavailable: {err}");
            }
            Ok(())
        }
        Some("cluster") => {
            ensure_no_extra_args(args)?;
            let snapshot = cluster::load_cluster_snapshot()?;
            println!("FOD monitor cluster");
            println!("version={FOD_VERSION}");
            println!("generated_unix_seconds={}", unix_seconds_now());
            cluster::print_cluster_snapshot(&snapshot, None);
            Ok(())
        }
        Some("top") | Some("watch") => {
            let options = parse_top_options(args)?;
            run_top(options)
        }
        Some("report") => {
            ensure_no_extra_args(args)?;
            let cluster_snapshot = cluster::load_cluster_snapshot();
            let snapshot = monitor_snapshot()?;
            print_report(&snapshot, cluster_snapshot.as_ref().ok());
            if let Err(err) = cluster_snapshot {
                eprintln!("FOD shared cluster telemetry unavailable: {err}");
            }
            Ok(())
        }
        Some("-h") | Some("--help") | Some("help") => {
            ensure_no_extra_args(args)?;
            print_help();
            Ok(())
        }
        Some("-V") | Some("--version") | Some("version") => {
            ensure_no_extra_args(args)?;
            println!("fod-monitor {FOD_VERSION}");
            Ok(())
        }
        Some(command) => Err(format!(
            "unknown fod-monitor command `{command}`; try `fod-monitor --help`"
        )),
    }
}

fn ensure_no_extra_args(mut args: impl Iterator<Item = String>) -> Result<(), String> {
    match args.next() {
        Some(arg) => Err(format!("unexpected argument `{arg}`")),
        None => Ok(()),
    }
}

fn parse_top_options(args: impl Iterator<Item = String>) -> Result<TopOptions, String> {
    let mut interval = Duration::from_secs(DEFAULT_TOP_INTERVAL_SECONDS);
    let mut iterations = None;
    let mut clear_screen = true;
    let mut args = args.peekable();

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--interval" => {
                let value = args
                    .next()
                    .ok_or_else(|| "missing value for --interval".to_string())?;
                let seconds = value
                    .parse::<u64>()
                    .map_err(|err| format!("invalid --interval value `{value}`: {err}"))?;
                if seconds == 0 {
                    return Err("--interval must be greater than zero".to_string());
                }
                interval = Duration::from_secs(seconds);
            }
            "--iterations" => {
                let value = args
                    .next()
                    .ok_or_else(|| "missing value for --iterations".to_string())?;
                let count = value
                    .parse::<u64>()
                    .map_err(|err| format!("invalid --iterations value `{value}`: {err}"))?;
                if count == 0 {
                    return Err("--iterations must be greater than zero".to_string());
                }
                iterations = Some(count);
            }
            "--no-clear" => clear_screen = false,
            "-h" | "--help" => {
                print_top_help();
                std::process::exit(0);
            }
            other => return Err(format!("unexpected top argument `{other}`")),
        }
    }

    Ok(TopOptions {
        interval,
        iterations,
        clear_screen,
    })
}

fn print_help() {
    println!("Observe current FOD runtime state.");
    println!();
    println!("Usage:");
    println!("  fod-monitor [status]");
    println!("  fod-monitor cluster");
    println!("  fod-monitor top [--interval SECONDS] [--iterations N] [--no-clear]");
    println!("  fod-monitor report");
    println!("  fod-monitor --help");
    println!("  fod-monitor --version");
    println!();
    println!("Commands:");
    println!("  status   Show shared cluster telemetry and local host diagnostics");
    println!("  cluster  Show centrally stored telemetry for all active FOD sessions");
    println!("  top      Refresh cluster and local status continuously, similarly to top");
    println!("  watch    Alias for top");
    println!("  report   Generate shared and local one-shot diagnostics");
}

fn print_top_help() {
    println!("Continuously observe current FOD runtime state.");
    println!();
    println!("Usage: fod-monitor top [--interval SECONDS] [--iterations N] [--no-clear]");
    println!();
    println!("Options:");
    println!("  --interval SECONDS   Refresh interval, default {DEFAULT_TOP_INTERVAL_SECONDS}");
    println!("  --iterations N       Stop after N refreshes; omit for continuous monitoring");
    println!("  --no-clear           Do not clear the terminal between refreshes");
}

fn run_top(options: TopOptions) -> Result<(), String> {
    let mut completed = 0_u64;
    let mut previous_cluster = None;
    loop {
        if options.clear_screen {
            print!("\x1b[2J\x1b[H");
        }

        let cluster_snapshot = cluster::load_cluster_snapshot();
        let snapshot = monitor_snapshot()?;
        println!(
            "FOD monitor top version={FOD_VERSION} generated_unix_seconds={}",
            unix_seconds_now()
        );
        match cluster_snapshot {
            Ok(current_cluster) => {
                cluster::print_cluster_snapshot(&current_cluster, previous_cluster.as_ref());
                println!();
                previous_cluster = Some(current_cluster);
            }
            Err(err) => {
                println!("Cluster:");
                println!("  unavailable={err}");
                println!();
            }
        }
        print_status_body(&snapshot);

        completed += 1;
        if options
            .iterations
            .is_some_and(|iterations| completed >= iterations)
        {
            return Ok(());
        }

        thread::sleep(options.interval);
    }
}

fn print_status(snapshot: &MonitorSnapshot, cluster_snapshot: Option<&cluster::ClusterSnapshot>) {
    println!("FOD monitor status");
    println!("version={FOD_VERSION}");
    println!("generated_unix_seconds={}", unix_seconds_now());
    if let Some(cluster_snapshot) = cluster_snapshot {
        cluster::print_cluster_snapshot(cluster_snapshot, None);
        println!();
    } else {
        println!("Cluster:");
        println!("  unavailable=true");
        println!();
    }
    print_status_body(snapshot);
}

fn print_report(snapshot: &MonitorSnapshot, cluster_snapshot: Option<&cluster::ClusterSnapshot>) {
    println!("FOD monitor report");
    println!("version={FOD_VERSION}");
    println!("generated_unix_seconds={}", unix_seconds_now());
    println!();
    if let Some(cluster_snapshot) = cluster_snapshot {
        cluster::print_cluster_snapshot(cluster_snapshot, None);
        println!();
        cluster::print_cluster_details(cluster_snapshot);
        println!();
    } else {
        println!("Cluster telemetry unavailable.");
        println!();
    }
    print_system_section(&snapshot.system);
    println!();
    print_process_summary(&snapshot.processes);
    println!();
    print_process_table(&snapshot.processes);
    println!();
    println!("Hints:");
    println!("  fod-monitor cluster");
    println!("  fod-monitor top --interval 2");
    println!("  fod-monitor top --iterations 5 --no-clear");
    println!("  fod-monitor report > fod-monitor-report.txt");
}

fn print_status_body(snapshot: &MonitorSnapshot) {
    print_system_section(&snapshot.system);
    print_process_summary(&snapshot.processes);
    print_process_table(&snapshot.processes);
}

fn print_system_section(system: &SystemSnapshot) {
    println!("System:");
    println!(
        "  load_average={}",
        system.load_average.as_deref().unwrap_or("unknown")
    );
    println!(
        "  uptime_seconds={}",
        option_u64(system.uptime_seconds.as_ref())
    );
    println!(
        "  mem_total_bytes={}",
        option_u64(system.mem_total_bytes.as_ref())
    );
    println!(
        "  mem_available_bytes={}",
        option_u64(system.mem_available_bytes.as_ref())
    );
}

fn print_process_summary(processes: &[ProcessSnapshot]) {
    let total_rss = processes
        .iter()
        .filter_map(|process| process.rss_bytes)
        .sum::<u64>();
    let total_threads = processes
        .iter()
        .filter_map(|process| process.threads)
        .sum::<u64>();
    let total_fds = processes
        .iter()
        .filter_map(|process| process.fd_count)
        .sum::<u64>();

    println!("FOD processes:");
    println!("  count={}", processes.len());
    println!("  total_rss_bytes={total_rss}");
    println!("  total_threads={total_threads}");
    println!("  total_fd_count={total_fds}");
}

fn print_process_table(processes: &[ProcessSnapshot]) {
    if processes.is_empty() {
        println!("No local FOD processes detected.");
        return;
    }

    println!("PID\tCOMMAND\tSTATE\tRSS_BYTES\tVM_SIZE_BYTES\tTHREADS\tFD_COUNT\tVOL_CTX\tINVOL_CTX\tCMDLINE");
    for process in processes {
        println!(
            "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
            process.pid,
            process.command,
            process.state,
            option_u64(process.rss_bytes.as_ref()),
            option_u64(process.vm_size_bytes.as_ref()),
            option_u64(process.threads.as_ref()),
            option_u64(process.fd_count.as_ref()),
            option_u64(process.voluntary_context_switches.as_ref()),
            option_u64(process.nonvoluntary_context_switches.as_ref()),
            process.cmdline
        );
    }
}

fn option_u64(value: Option<&u64>) -> String {
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| "unknown".to_string())
}

fn monitor_snapshot() -> Result<MonitorSnapshot, String> {
    Ok(MonitorSnapshot {
        system: system_snapshot(),
        processes: fod_process_snapshots()?,
    })
}

fn system_snapshot() -> SystemSnapshot {
    SystemSnapshot {
        load_average: fs::read_to_string("/proc/loadavg")
            .ok()
            .map(|value| value.trim().to_string()),
        uptime_seconds: fs::read_to_string("/proc/uptime")
            .ok()
            .and_then(|value| value.split_whitespace().next()?.parse::<f64>().ok())
            .map(|seconds| seconds as u64),
        mem_total_bytes: meminfo_bytes("MemTotal"),
        mem_available_bytes: meminfo_bytes("MemAvailable"),
    }
}

fn meminfo_bytes(field: &str) -> Option<u64> {
    let meminfo = fs::read_to_string("/proc/meminfo").ok()?;
    meminfo
        .lines()
        .find_map(|line| line.strip_prefix(&format!("{field}:")))
        .and_then(|value| parse_kib_bytes(value.trim()).ok())
}

fn fod_process_snapshots() -> Result<Vec<ProcessSnapshot>, String> {
    let proc_dir = fs::read_dir("/proc").map_err(|err| format!("unable to read /proc: {err}"))?;
    let current_pid = std::process::id();
    let mut processes = Vec::new();

    for entry in proc_dir {
        let entry = entry.map_err(|err| format!("unable to read /proc entry: {err}"))?;
        let file_name = entry.file_name();
        let Some(pid) = file_name.to_string_lossy().parse::<u32>().ok() else {
            continue;
        };
        if pid == current_pid {
            continue;
        }

        let Ok(snapshot) = process_snapshot(pid) else {
            continue;
        };
        if FOD_PROCESS_NAMES
            .iter()
            .any(|name| snapshot.command == *name || snapshot.command.starts_with(name))
        {
            processes.push(snapshot);
        }
    }

    processes.sort_by_key(|process| process.pid);
    Ok(processes)
}

fn process_snapshot(pid: u32) -> Result<ProcessSnapshot, String> {
    let status_path = format!("/proc/{pid}/status");
    let status = fs::read_to_string(&status_path)
        .map_err(|err| format!("unable to read {status_path}: {err}"))?;
    let command = status_field(&status, "Name").unwrap_or_else(|| "unknown".to_string());
    let state = status_field(&status, "State").unwrap_or_else(|| "unknown".to_string());
    let rss_bytes = status_field(&status, "VmRSS").and_then(|value| parse_kib_bytes(&value).ok());
    let vm_size_bytes =
        status_field(&status, "VmSize").and_then(|value| parse_kib_bytes(&value).ok());
    let threads = status_field(&status, "Threads").and_then(|value| value.parse::<u64>().ok());
    let voluntary_context_switches = status_field(&status, "voluntary_ctxt_switches")
        .and_then(|value| value.parse::<u64>().ok());
    let nonvoluntary_context_switches = status_field(&status, "nonvoluntary_ctxt_switches")
        .and_then(|value| value.parse::<u64>().ok());
    let fd_count = process_fd_count(pid);
    let cmdline = process_cmdline(pid).unwrap_or_else(|| command.clone());

    Ok(ProcessSnapshot {
        pid,
        command,
        state,
        rss_bytes,
        vm_size_bytes,
        threads,
        fd_count,
        voluntary_context_switches,
        nonvoluntary_context_switches,
        cmdline,
    })
}

fn process_fd_count(pid: u32) -> Option<u64> {
    fs::read_dir(format!("/proc/{pid}/fd"))
        .ok()
        .map(|entries| entries.filter_map(Result::ok).count() as u64)
}

fn process_cmdline(pid: u32) -> Option<String> {
    let bytes = fs::read(format!("/proc/{pid}/cmdline")).ok()?;
    if bytes.is_empty() {
        return None;
    }
    let value = bytes
        .split(|byte| *byte == 0)
        .filter(|part| !part.is_empty())
        .map(|part| String::from_utf8_lossy(part))
        .collect::<Vec<_>>()
        .join(" ");
    if value.is_empty() {
        None
    } else {
        Some(truncate_chars(&value, MAX_CMDLINE_CHARS))
    }
}

fn truncate_chars(value: &str, max_chars: usize) -> String {
    let mut chars = value.chars();
    let truncated = chars.by_ref().take(max_chars).collect::<String>();
    if chars.next().is_some() {
        format!("{truncated}...")
    } else {
        truncated
    }
}

fn status_field(status: &str, field: &str) -> Option<String> {
    let prefix = format!("{field}:");
    status
        .lines()
        .find_map(|line| line.strip_prefix(&prefix))
        .map(|value| value.trim().to_string())
}

fn parse_kib_bytes(value: &str) -> Result<u64, String> {
    let mut parts = value.split_whitespace();
    let kib = parts
        .next()
        .ok_or_else(|| "missing KiB value".to_string())?
        .parse::<u64>()
        .map_err(|err| format!("invalid KiB value: {err}"))?;
    let unit = parts.next().ok_or_else(|| "missing KiB unit".to_string())?;
    if unit != "kB" {
        return Err(format!("unsupported unit: {unit}"));
    }
    kib.checked_mul(1024)
        .ok_or_else(|| "byte value overflowed".to_string())
}

fn unix_seconds_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
