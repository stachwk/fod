// Copyright (c) 2026 Wojciech Stach
// Licensed under BSL 1.1

#[path = "../config.rs"]
mod config;
#[path = "../pg.rs"]
mod pg;
#[path = "../pg_config.rs"]
mod pg_config;
#[path = "../tls.rs"]
mod tls;
#[path = "../version.rs"]
mod version;

use clap::{Parser, Subcommand};
use config::{load_config_parser, resolve_config_path};
use pg::DbConn;
use serde_json::json;
use std::path::{Path, PathBuf};
use tls::generate_client_tls_pair;

#[derive(Parser)]
#[command(name = "fod-config", about = "Resolve FOD config and TLS helpers.")]
struct Cli {
    #[arg(long)]
    config_path: Option<PathBuf>,
    #[arg(long)]
    base_dir: Option<PathBuf>,
    #[command(subcommand)]
    command: CommandKind,
}

#[derive(Subcommand)]
enum CommandKind {
    ResolvePath,
    ConnectionParams,
    EndpointConfig,
    EndpointProbe,
    RuntimeConfig,
    Version,
    GenerateTls {
        #[arg(long, default_value = ".fod/tls")]
        material_dir: PathBuf,
        #[arg(
            long,
            default_value = "fod",
            help = "TLS common name for generated client material; allowed chars: ASCII letters, digits, dot, underscore, hyphen."
        )]
        common_name: String,
        #[arg(long, default_value_t = 365)]
        days: i64,
    },
}

fn main() {
    let cli = Cli::parse();
    let config_path = match resolve_config_path(cli.config_path.as_deref()) {
        Ok(path) => path,
        Err(err) => {
            eprintln!("{}", err);
            std::process::exit(1);
        }
    };
    let (config, config_path) = match load_config_parser(Some(&config_path)) {
        Ok(value) => value,
        Err(err) => {
            eprintln!("{}", err);
            std::process::exit(1);
        }
    };
    match cli.command {
        CommandKind::ResolvePath => {
            println!("{}", config_path.display());
        }
        CommandKind::ConnectionParams => {
            let db_section = match database_section(&config) {
                Ok(section) => section,
                Err(err) => exit_with_error(&err),
            };
            let params = pg_config::resolve_pg_connection_params(
                &db_section,
                &config_path.parent().unwrap_or(Path::new(".")),
            );
            let mut map = serde_json::Map::new();
            for (key, value) in params {
                map.insert(key, serde_json::Value::String(value));
            }
            println!("{}", serde_json::Value::Object(map));
        }
        CommandKind::EndpointConfig => {
            let db_section = match database_section(&config) {
                Ok(section) => section,
                Err(err) => exit_with_error(&err),
            };
            let topology = match pg_config::resolve_pg_endpoint_config(&db_section) {
                Ok(value) => value,
                Err(err) => exit_with_error(&err),
            };
            let endpoints = topology
                .endpoints
                .iter()
                .map(|endpoint| {
                    json!({
                        "host": endpoint.host.as_str(),
                        "port": endpoint.port,
                        "role": endpoint.role.as_str(),
                        "authority": endpoint.authority(),
                    })
                })
                .collect::<Vec<_>>();
            println!(
                "{}",
                json!({
                    "mode": topology.mode.as_str(),
                    "routing_enabled": false,
                    "role_discovery_required": topology.role_discovery_required,
                    "primary_count": topology.primary_count(),
                    "replica_count": topology.replica_count(),
                    "unknown_count": topology.unknown_count(),
                    "endpoints": endpoints,
                })
            );
        }
        CommandKind::EndpointProbe => {
            let db_section = match database_section(&config) {
                Ok(section) => section,
                Err(err) => exit_with_error(&err),
            };
            let topology = match pg_config::resolve_pg_endpoint_config(&db_section) {
                Ok(value) => value,
                Err(err) => exit_with_error(&err),
            };
            let base_params = pg_config::resolve_pg_connection_params(
                &db_section,
                &config_path.parent().unwrap_or(Path::new(".")),
            );
            let mut reachable_count = 0usize;
            let mut failed_count = 0usize;
            let mut write_capable_count = 0usize;
            let mut role_mismatch_count = 0usize;
            let mut inconsistent_count = 0usize;
            let mut endpoints = Vec::new();

            for endpoint in &topology.endpoints {
                let params = pg_config::pg_connection_params_for_endpoint(&base_params, endpoint);
                let conninfo = pg_config::make_conninfo(&params);
                match DbConn::connect(&conninfo) {
                    Ok(conn) => match conn.postgres_endpoint_probe() {
                        Ok(probe) => {
                            reachable_count += 1;
                            if probe.write_capable() {
                                write_capable_count += 1;
                            }
                            if !probe.is_consistent() {
                                inconsistent_count += 1;
                            }
                            let role_matches = probe.configured_role_matches(endpoint.role);
                            if role_matches == Some(false) {
                                role_mismatch_count += 1;
                            }
                            endpoints.push(json!({
                                "authority": endpoint.authority(),
                                "host": endpoint.host.as_str(),
                                "port": endpoint.port,
                                "configured_role": endpoint.role.as_str(),
                                "connected": true,
                                "pg_is_in_recovery": probe.pg_is_in_recovery,
                                "transaction_read_only": probe.transaction_read_only,
                                "observed_role": probe.observed_role.as_str(),
                                "write_capable": probe.write_capable(),
                                "consistent": probe.is_consistent(),
                                "role_matches_config": role_matches,
                                "error": null,
                            }));
                        }
                        Err(err) => {
                            failed_count += 1;
                            endpoints.push(json!({
                                "authority": endpoint.authority(),
                                "host": endpoint.host.as_str(),
                                "port": endpoint.port,
                                "configured_role": endpoint.role.as_str(),
                                "connected": true,
                                "observed_role": null,
                                "write_capable": false,
                                "consistent": false,
                                "role_matches_config": null,
                                "error": err,
                            }));
                        }
                    },
                    Err(err) => {
                        failed_count += 1;
                        endpoints.push(json!({
                            "authority": endpoint.authority(),
                            "host": endpoint.host.as_str(),
                            "port": endpoint.port,
                            "configured_role": endpoint.role.as_str(),
                            "connected": false,
                            "observed_role": null,
                            "write_capable": false,
                            "consistent": false,
                            "role_matches_config": null,
                            "error": err,
                        }));
                    }
                }
            }

            println!(
                "{}",
                json!({
                    "mode": topology.mode.as_str(),
                    "routing_enabled": false,
                    "probe_only": true,
                    "endpoint_count": topology.endpoints.len(),
                    "reachable_count": reachable_count,
                    "failed_count": failed_count,
                    "write_capable_count": write_capable_count,
                    "role_mismatch_count": role_mismatch_count,
                    "inconsistent_count": inconsistent_count,
                    "all_probes_succeeded": failed_count == 0,
                    "endpoints": endpoints,
                })
            );
        }
        CommandKind::RuntimeConfig => {
            let runtime = match config::load_runtime_config(&config) {
                Ok(value) => value,
                Err(err) => exit_with_error(&err),
            };
            let mut map = serde_json::Map::new();
            for (key, value) in runtime.to_runtime_map() {
                map.insert(key, serde_json::Value::String(value));
            }
            println!("{}", serde_json::Value::Object(map));
        }
        CommandKind::Version => {
            println!("{}", version::FOD_VERSION_LABEL);
        }
        CommandKind::GenerateTls {
            material_dir,
            common_name,
            days,
        } => match generate_client_tls_pair(&material_dir, &common_name, days) {
            Ok((cert_path, key_path)) => {
                println!(
                    "{}",
                    json!({"cert_path": cert_path.display().to_string(), "key_path": key_path.display().to_string()})
                );
            }
            Err(err) => exit_with_error(&err),
        },
    }
}

fn database_section(
    config: &config::IniConfig,
) -> Result<std::collections::HashMap<String, String>, String> {
    config
        .section("database")
        .cloned()
        .ok_or_else(|| "Missing [database] section in FOD configuration".to_string())
}

fn exit_with_error(message: &str) -> ! {
    eprintln!("{}", message);
    std::process::exit(1);
}
