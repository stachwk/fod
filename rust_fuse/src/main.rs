// Copyright (c) 2026 Wojciech Stach
// Licensed under BSL 1.1

mod compatibility;
mod copy_plan;
mod fs;
mod read_cache;
mod startup;
mod write_buffer;
mod write_payload;

use clap::Parser;
use fod_rust_runtime::{env_var_with_legacy_alias, RuntimeConfig};
use log::LevelFilter;
use rust_hotpath::pg::DbRepo;
use std::io::Write;
use std::path::PathBuf;

#[derive(Parser, Debug)]
struct Args {
    #[arg(short = 'f', long = "mountpoint")]
    mountpoint: PathBuf,
    #[arg(long = "readonly", default_value_t = false)]
    readonly: bool,
}

fn init_logging() {
    let level = env_var_with_legacy_alias("FOD_LOG_LEVEL").unwrap_or_else(|| "info".to_string());
    let filter = level.parse::<LevelFilter>().unwrap_or(LevelFilter::Info);
    let mut builder = env_logger::Builder::new();
    builder.filter_level(filter);
    builder.format(|buf, record| {
        writeln!(
            buf,
            "{} - {} - {}",
            buf.timestamp_seconds(),
            record.level(),
            record.args()
        )
    });
    if std::env::var_os("RUST_LOG").is_none() {
        builder.parse_filters(&level);
    }
    builder.init();
}

fn main() {
    init_logging();
    let args = Args::parse();
    let conninfo = env_var_with_legacy_alias("FOD_DSN_CONNINFO")
        .expect("FOD_DSN_CONNINFO must be set when launching fod-rust-fuse");
    log::debug!("FOD resolved mountpoint={}", args.mountpoint.display());
    log::debug!("FOD creating PostgreSQL repo");
    let runtime = RuntimeConfig::from_env().unwrap_or_else(|err| {
        eprintln!("fod-rust-fuse: invalid runtime config: {err}");
        std::process::exit(1);
    });
    let repo = DbRepo::with_runtime(&conninfo, &runtime).unwrap_or_else(|err| {
        eprintln!("fod-rust-fuse: failed to open PostgreSQL repo: {err}");
        std::process::exit(1);
    });
    match repo.postgres_version_diagnostics() {
        Ok(postgres_versions) => log::info!(
            "FOD PostgreSQL diagnostics: libpq={} ({}) server={} ({}) major_relation={} compatibility={}",
            postgres_versions.libpq_version,
            postgres_versions.libpq_version_num,
            postgres_versions.server_version,
            postgres_versions.server_version_num,
            postgres_versions.major_relation,
            postgres_versions.compatibility_label()
        ),
        Err(err) => log::warn!(
            "FOD PostgreSQL diagnostics unavailable; continuing with normal startup checks: {}",
            err
        ),
    }
    log::debug!("FOD reading startup snapshot");
    let snapshot = repo.startup_snapshot().unwrap_or_else(|err| {
        eprintln!("fod-rust-fuse: failed to read startup snapshot: {err}");
        std::process::exit(1);
    });
    log::debug!("FOD startup snapshot={:?}", snapshot);
    let settings = startup::FodFuseSettings::from_runtime(&runtime, &snapshot, args.readonly);
    if let Err(err) = startup::mount_fuse(repo, &runtime, settings, &args.mountpoint, &snapshot) {
        log::error!("FOD mount failed: {}", err);
        eprintln!("fod-rust-fuse: {}", err);
        std::process::exit(1);
    }
}
