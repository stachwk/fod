// Copyright (c) 2026 Wojciech Stach
// Licensed under BSL 1.1

#[path = "../config.rs"]
mod config;
#[path = "../pg_config.rs"]
mod pg_config;
#[path = "../tls.rs"]
mod tls;
#[path = "../version.rs"]
mod version;

use clap::{Parser, Subcommand};
use config::{load_config_parser, resolve_config_path};
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
            let db_section = match config.section("database") {
                Some(section) => section.clone(),
                None => {
                    eprintln!("Missing [database] section in FOD configuration");
                    std::process::exit(1);
                }
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
        CommandKind::RuntimeConfig => {
            let runtime = match config::load_runtime_config(&config) {
                Ok(value) => value,
                Err(err) => {
                    eprintln!("{}", err);
                    std::process::exit(1);
                }
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
            Err(err) => {
                eprintln!("{}", err);
                std::process::exit(1);
            }
        },
    }
}
