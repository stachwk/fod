// Copyright (c) 2026 Wojciech Stach
// Licensed under BSL 1.1

mod cluster;

use std::collections::BTreeSet;
use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::Serialize;

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
const CLUSTER_JSON_SCHEMA_VERSION: u32 = 1;
const REPORT_JSON_SCHEMA_VERSION: u32 = 3;
const MKFS_BIN_ENV: &str = "FOD_MKFS_BIN";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct SystemSnapshot {
    load_average: Option<String>,
    uptime_seconds: Option<u64>,
    mem_total_bytes: Option<u64>,
    mem_available_bytes: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct MonitorSnapshot {
    system: SystemSnapshot,
    processes: Vec<ProcessSnapshot>,
}

#[derive(Debug, Serialize)]
struct ClusterCommandJson<'a> {
    schema_version: u32,
    fod_version: &'a str,
    generated_unix_seconds: u64,
    cluster: cluster::ClusterJsonSnapshot<'a>,
}

#[derive(Debug, Serialize)]
struct ReportCommandJson<'a> {
    schema_version: u32,
    fod_version: &'a str,
    generated_unix_seconds: u64,
    compatibility_summary: CompatibilitySummary,
    mkfs_status: Option<serde_json::Value>,
    mkfs_status_error: Option<String>,
    cluster: Option<cluster::ClusterJsonSnapshot<'a>>,
    cluster_error: Option<String>,
    local: &'a MonitorSnapshot,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct CompatibilitySummary {
    coverage: String,
    postgresql: CompatibilityPostgresqlSummary,
    fuse: CompatibilityFuseSummary,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct CompatibilityPostgresqlSummary {
    source_status: String,
    compatibility: Option<String>,
    libpq_version_num: Option<u64>,
    server_version_num: Option<u64>,
    minimum_server_version_num: Option<u64>,
    server_configuration_warning_count: Option<u64>,
    session_configuration_error_count: Option<u64>,
    schema_ready: Option<bool>,
    schema_version: Option<u64>,
    latest_migration_version: Option<u64>,
    pending_migration_count: Option<u64>,
    storage_block_size_bytes: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct CompatibilityFuseSummary {
    source_status: String,
    active_sessions: u64,
    telemetry_sessions: u64,
    negotiated_sessions: u64,
    missing_negotiation_sessions: u64,
    shared_monitor_schema_versions: Vec<u32>,
    fuser_versions: Vec<String>,
    kernel_protocols: Vec<String>,
    negotiated_protocols: Vec<String>,
    unsupported_requested_capabilities: Vec<String>,
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
            let json = parse_json_flag(args)?;
            let snapshot = cluster::load_cluster_snapshot()?;
            let generated_unix_seconds = unix_seconds_now();
            if json {
                print_json(&ClusterCommandJson {
                    schema_version: CLUSTER_JSON_SCHEMA_VERSION,
                    fod_version: FOD_VERSION,
                    generated_unix_seconds,
                    cluster: cluster::cluster_json_snapshot(&snapshot),
                })?;
            } else {
                println!("FOD monitor cluster");
                println!("version={FOD_VERSION}");
                println!("generated_unix_seconds={generated_unix_seconds}");
                cluster::print_cluster_snapshot(&snapshot, None);
            }
            Ok(())
        }
        Some("top") | Some("watch") => {
            let options = parse_top_options(args)?;
            run_top(options)
        }
        Some("report") => {
            let json = parse_json_flag(args)?;
            let cluster_snapshot = cluster::load_cluster_snapshot();
            let snapshot = monitor_snapshot()?;
            if json {
                let mkfs_status = load_mkfs_status_json();
                print_report_json(&snapshot, cluster_snapshot, mkfs_status)?;
            } else {
                print_report(&snapshot, cluster_snapshot.as_ref().ok());
                if let Err(err) = cluster_snapshot {
                    eprintln!("FOD shared cluster telemetry unavailable: {err}");
                }
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

fn parse_json_flag(args: impl Iterator<Item = String>) -> Result<bool, String> {
    let mut json = false;
    for arg in args {
        match arg.as_str() {
            "--json" => json = true,
            "-h" | "--help" => {
                print_help();
                std::process::exit(0);
            }
            other => return Err(format!("unexpected argument `{other}`")),
        }
    }
    Ok(json)
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
    println!("  fod-monitor cluster [--json]");
    println!("  fod-monitor top [--interval SECONDS] [--iterations N] [--no-clear]");
    println!("  fod-monitor report [--json]");
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

fn print_report_json(
    snapshot: &MonitorSnapshot,
    cluster_snapshot: Result<cluster::ClusterSnapshot, String>,
    mkfs_status: Result<serde_json::Value, String>,
) -> Result<(), String> {
    let generated_unix_seconds = unix_seconds_now();

    let cluster_error = cluster_snapshot.as_ref().err().cloned();
    let cluster_ref = cluster_snapshot.as_ref().ok();
    let cluster = cluster_ref.map(cluster::cluster_json_snapshot);

    let (mkfs_status, mkfs_status_error) = match mkfs_status {
        Ok(status) => (Some(status), None),
        Err(err) => (None, Some(err)),
    };

    let compatibility_summary = compatibility_summary(mkfs_status.as_ref(), cluster_ref);

    print_json(&ReportCommandJson {
        schema_version: REPORT_JSON_SCHEMA_VERSION,
        fod_version: FOD_VERSION,
        generated_unix_seconds,
        compatibility_summary,
        mkfs_status,
        mkfs_status_error,
        cluster,
        cluster_error,
        local: snapshot,
    })
}

fn json_path<'a>(value: &'a serde_json::Value, path: &[&str]) -> Option<&'a serde_json::Value> {
    let mut current = value;
    for key in path {
        current = current.get(*key)?;
    }
    Some(current)
}

fn json_string_path(value: &serde_json::Value, path: &[&str]) -> Option<String> {
    json_path(value, path)?.as_str().map(ToString::to_string)
}

fn json_u64_path(value: &serde_json::Value, path: &[&str]) -> Option<u64> {
    json_path(value, path)?.as_u64()
}

fn json_bool_path(value: &serde_json::Value, path: &[&str]) -> Option<bool> {
    json_path(value, path)?.as_bool()
}

fn json_array_len_path(value: &serde_json::Value, path: &[&str]) -> Option<u64> {
    let len = json_path(value, path)?.as_array()?.len();
    u64::try_from(len).ok()
}

fn compatibility_postgresql_summary(
    mkfs_status: Option<&serde_json::Value>,
) -> CompatibilityPostgresqlSummary {
    let Some(status) = mkfs_status else {
        return CompatibilityPostgresqlSummary {
            source_status: "unavailable".to_string(),
            compatibility: None,
            libpq_version_num: None,
            server_version_num: None,
            minimum_server_version_num: None,
            server_configuration_warning_count: None,
            session_configuration_error_count: None,
            schema_ready: None,
            schema_version: None,
            latest_migration_version: None,
            pending_migration_count: None,
            storage_block_size_bytes: None,
        };
    };

    let compatibility = json_string_path(status, &["postgresql", "compatibility"]);
    let libpq_version_num = json_u64_path(status, &["postgresql", "libpq_runtime", "version_num"]);
    let server_version_num =
        json_u64_path(status, &["postgresql", "server_runtime", "version_num"]);
    let minimum_server_version_num = json_u64_path(
        status,
        &["postgresql", "requirements", "minimum_server_version_num"],
    );
    let server_configuration_warning_count = json_array_len_path(
        status,
        &[
            "postgresql",
            "requirements",
            "server_configuration_warnings",
        ],
    );
    let session_configuration_error_count = json_array_len_path(
        status,
        &["postgresql", "requirements", "session_configuration_errors"],
    );
    let schema_ready = json_bool_path(status, &["schema", "ready"]);
    let schema_version = json_u64_path(status, &["schema", "version"]);
    let latest_migration_version = json_u64_path(status, &["schema", "latest_migration_version"]);
    let pending_migration_count = json_array_len_path(status, &["schema", "pending_migrations"]);
    let storage_block_size_bytes = json_u64_path(status, &["storage_format", "block_size_bytes"]);

    let complete = compatibility.is_some()
        && libpq_version_num.is_some()
        && server_version_num.is_some()
        && minimum_server_version_num.is_some()
        && server_configuration_warning_count.is_some()
        && session_configuration_error_count.is_some()
        && schema_ready.is_some()
        && schema_version.is_some()
        && latest_migration_version.is_some()
        && pending_migration_count.is_some()
        && storage_block_size_bytes.is_some();

    CompatibilityPostgresqlSummary {
        source_status: if complete {
            "available".to_string()
        } else {
            "partial".to_string()
        },
        compatibility,
        libpq_version_num,
        server_version_num,
        minimum_server_version_num,
        server_configuration_warning_count,
        session_configuration_error_count,
        schema_ready,
        schema_version,
        latest_migration_version,
        pending_migration_count,
        storage_block_size_bytes,
    }
}

fn compatibility_fuse_summary(
    cluster_snapshot: Option<&cluster::ClusterSnapshot>,
) -> CompatibilityFuseSummary {
    let Some(cluster_snapshot) = cluster_snapshot else {
        return CompatibilityFuseSummary {
            source_status: "unavailable".to_string(),
            active_sessions: 0,
            telemetry_sessions: 0,
            negotiated_sessions: 0,
            missing_negotiation_sessions: 0,
            shared_monitor_schema_versions: Vec::new(),
            fuser_versions: Vec::new(),
            kernel_protocols: Vec::new(),
            negotiated_protocols: Vec::new(),
            unsupported_requested_capabilities: Vec::new(),
        };
    };

    let active_sessions = cluster_snapshot.sessions.len() as u64;
    let telemetry_sessions = cluster_snapshot
        .sessions
        .iter()
        .filter(|session| session.stats.is_some())
        .count() as u64;
    let negotiated_sessions = cluster_snapshot
        .sessions
        .iter()
        .filter(|session| {
            session
                .stats
                .as_ref()
                .and_then(|stats| stats.fuse_compatibility.as_ref())
                .is_some()
        })
        .count() as u64;

    let mut shared_monitor_schema_versions = BTreeSet::new();
    let mut fuser_versions = BTreeSet::new();
    let mut kernel_protocols = BTreeSet::new();
    let mut negotiated_protocols = BTreeSet::new();
    let mut unsupported_requested_capabilities = BTreeSet::new();

    for session in &cluster_snapshot.sessions {
        let Some(stats) = session.stats.as_ref() else {
            continue;
        };
        shared_monitor_schema_versions.insert(stats.schema_version);
        let Some(fuse) = stats.fuse_compatibility.as_ref() else {
            continue;
        };
        if !fuse.fuser_version.is_empty() {
            fuser_versions.insert(fuse.fuser_version.clone());
        }
        if !fuse.kernel_protocol.is_empty() {
            kernel_protocols.insert(fuse.kernel_protocol.clone());
        }
        if !fuse.negotiated_protocol.is_empty() {
            negotiated_protocols.insert(fuse.negotiated_protocol.clone());
        }
        unsupported_requested_capabilities.extend(fuse.unsupported_capabilities.iter().cloned());
    }

    let source_status = if active_sessions == 0 {
        "no_active_sessions"
    } else if negotiated_sessions == active_sessions {
        "available"
    } else if negotiated_sessions > 0 {
        "partial"
    } else {
        "unavailable"
    };

    CompatibilityFuseSummary {
        source_status: source_status.to_string(),
        active_sessions,
        telemetry_sessions,
        negotiated_sessions,
        missing_negotiation_sessions: active_sessions.saturating_sub(negotiated_sessions),
        shared_monitor_schema_versions: shared_monitor_schema_versions.into_iter().collect(),
        fuser_versions: fuser_versions.into_iter().collect(),
        kernel_protocols: kernel_protocols.into_iter().collect(),
        negotiated_protocols: negotiated_protocols.into_iter().collect(),
        unsupported_requested_capabilities: unsupported_requested_capabilities
            .into_iter()
            .collect(),
    }
}

fn compatibility_summary(
    mkfs_status: Option<&serde_json::Value>,
    cluster_snapshot: Option<&cluster::ClusterSnapshot>,
) -> CompatibilitySummary {
    let postgresql = compatibility_postgresql_summary(mkfs_status);
    let fuse = compatibility_fuse_summary(cluster_snapshot);

    let postgresql_complete = postgresql.source_status == "available";
    let fuse_complete = matches!(
        fuse.source_status.as_str(),
        "available" | "no_active_sessions"
    );
    let any_source_available =
        postgresql.source_status != "unavailable" || cluster_snapshot.is_some();

    let coverage = if postgresql_complete && fuse_complete {
        "complete"
    } else if any_source_available {
        "partial"
    } else {
        "unavailable"
    };

    CompatibilitySummary {
        coverage: coverage.to_string(),
        postgresql,
        fuse,
    }
}

fn mkfs_status_binary() -> Result<PathBuf, String> {
    if let Some(configured) = env::var_os(MKFS_BIN_ENV) {
        if !configured.is_empty() {
            return Ok(PathBuf::from(configured));
        }
    }

    if let Ok(current_exe) = env::current_exe() {
        if let Some(parent) = current_exe.parent() {
            let sibling = parent.join("fod-rust-mkfs");
            if sibling.is_file() {
                return Ok(sibling);
            }
        }
    }

    Ok(PathBuf::from("fod-rust-mkfs"))
}

fn load_mkfs_status_json() -> Result<serde_json::Value, String> {
    let binary = mkfs_status_binary()?;
    let output = Command::new(&binary)
        .args(["status", "--json"])
        .output()
        .map_err(|err| {
            format!(
                "unable to execute {} status --json: {err}",
                binary.display()
            )
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(format!(
            "{} status --json failed status={} stderr={}",
            binary.display(),
            output.status,
            if stderr.is_empty() {
                "<empty>"
            } else {
                &stderr
            }
        ));
    }

    let payload: serde_json::Value = serde_json::from_slice(&output.stdout).map_err(|err| {
        format!(
            "{} status --json returned invalid JSON: {err}",
            binary.display()
        )
    })?;

    if payload
        .get("schema_version")
        .and_then(serde_json::Value::as_u64)
        .is_none()
    {
        return Err(format!(
            "{} status --json returned an unversioned payload",
            binary.display()
        ));
    }

    Ok(payload)
}

fn print_json<T: Serialize>(value: &T) -> Result<(), String> {
    let payload = serde_json::to_string_pretty(value)
        .map_err(|err| format!("unable to serialize monitor JSON: {err}"))?;
    println!("{payload}");
    Ok(())
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
