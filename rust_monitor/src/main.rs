// Copyright (c) 2026 Wojciech Stach
// Licensed under BSL 1.1

use std::env;
use std::fs;

const FOD_VERSION: &str = env!("CARGO_PKG_VERSION");
const FOD_PROCESS_NAMES: &[&str] = &[
    "fod-rust-fuse",
    "fod-indexer",
    "fod-bootstrap",
    "fod-change",
    "mkfs.fod",
];

#[derive(Debug, Clone, PartialEq, Eq)]
struct ProcessSnapshot {
    pid: u32,
    command: String,
    state: String,
    rss_bytes: Option<u64>,
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
            print_status()
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

fn print_help() {
    println!("Observe current FOD runtime state.");
    println!();
    println!("Usage: fod-monitor [status]");
    println!("       fod-monitor --help");
    println!("       fod-monitor --version");
    println!();
    println!("Commands:");
    println!("  status   Show local FOD processes and RSS memory usage");
}

fn print_status() -> Result<(), String> {
    let processes = fod_process_snapshots()?;
    println!("FOD monitor status");
    println!("version={FOD_VERSION}");
    println!("processes={}", processes.len());
    if processes.is_empty() {
        println!("No local FOD processes detected.");
        return Ok(());
    }

    println!("PID\tCOMMAND\tSTATE\tRSS_BYTES");
    for process in processes {
        let rss = process
            .rss_bytes
            .map(|bytes| bytes.to_string())
            .unwrap_or_else(|| "unknown".to_string());
        println!(
            "{}\t{}\t{}\t{}",
            process.pid, process.command, process.state, rss
        );
    }
    Ok(())
}

fn fod_process_snapshots() -> Result<Vec<ProcessSnapshot>, String> {
    let proc_dir = fs::read_dir("/proc").map_err(|err| format!("unable to read /proc: {err}"))?;
    let mut processes = Vec::new();
    for entry in proc_dir {
        let entry = entry.map_err(|err| format!("unable to read /proc entry: {err}"))?;
        let file_name = entry.file_name();
        let Some(pid) = file_name.to_string_lossy().parse::<u32>().ok() else {
            continue;
        };
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
    Ok(ProcessSnapshot {
        pid,
        command,
        state,
        rss_bytes,
    })
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
        .ok_or_else(|| "RSS byte value overflowed".to_string())
}
