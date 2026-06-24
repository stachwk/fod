use clap::{Parser, Subcommand, ValueEnum};
use fod_rust_runtime::FOD_VERSION_LABEL;
use std::ffi::OsString;

#[derive(Debug, Clone, Parser)]
#[command(
    name = "fod-indexer",
    version = FOD_VERSION_LABEL,
    about = "Index external files before importing them into FOD."
)]
pub struct Cli {
    #[arg(long)]
    pub conninfo: Option<String>,
    #[command(subcommand)]
    pub command: Commands,
}

impl Cli {
    pub fn parse_with_source_aliases() -> Self {
        Self::parse_from(normalize_indexer_args(std::env::args_os()))
    }
}

#[derive(Debug, Clone, Subcommand)]
pub enum Commands {
    Source {
        #[command(subcommand)]
        command: SourceCommands,
    },
    Scan {
        #[arg(long)]
        source: String,
    },
    Hash {
        #[arg(long)]
        source: String,
        #[arg(long, default_value_t = false)]
        candidates_only: bool,
    },
    Report {
        #[command(subcommand)]
        command: ReportCommands,
    },
    PlanImport {
        #[arg(long)]
        source: Option<String>,
        #[arg(long, default_value_t = false)]
        all_sources: bool,
        #[arg(long, default_value_t = false)]
        dry_run: bool,
    },
    CleanupFailed {
        #[arg(long)]
        plan: u64,
    },
    Materialize {
        #[arg(long)]
        source: String,
    },
}

#[derive(Debug, Clone, Subcommand)]
pub enum SourceCommands {
    Add {
        #[arg(long)]
        name: String,
        #[arg(long)]
        path: String,
        #[arg(long, value_enum, default_value_t = SourceKind::Local)]
        kind: SourceKind,
    },
}

#[derive(Debug, Clone, Subcommand)]
pub enum ReportCommands {
    Duplicates {
        #[arg(long, default_value_t = 100)]
        limit: usize,
    },
}

#[derive(Debug, Copy, Clone, Eq, PartialEq, ValueEnum)]
pub enum SourceKind {
    Local,
}

impl SourceKind {
    pub fn as_str(self) -> &'static str {
        match self {
            SourceKind::Local => "local",
        }
    }
}

fn normalize_indexer_args(args: impl IntoIterator<Item = OsString>) -> Vec<OsString> {
    let mut args = args.into_iter().collect::<Vec<_>>();
    let Some(command_index) = find_command_index(&args) else {
        return args;
    };
    let command = args[command_index].to_string_lossy().to_string();
    if !command_accepts_positional_source(&command) {
        return args;
    }
    if has_explicit_source_option(&args, command_index + 1) {
        return args;
    }
    if let Some(source_index) = find_positional_source_index(&args, command_index + 1) {
        args.insert(source_index, OsString::from("--source"));
    }
    args
}

fn find_command_index(args: &[OsString]) -> Option<usize> {
    let mut idx = 1usize;
    while idx < args.len() {
        let token = args[idx].to_string_lossy();
        match token.as_ref() {
            "--conninfo" => {
                idx = idx.saturating_add(2);
            }
            "-h" | "--help" | "-V" | "--version" => {
                idx += 1;
            }
            "scan" | "hash" | "plan-import" | "materialize" => return Some(idx),
            "source" | "report" | "cleanup-failed" => return None,
            _ if token.starts_with('-') => {
                idx += 1;
            }
            _ => return None,
        }
    }
    None
}

fn command_accepts_positional_source(command: &str) -> bool {
    matches!(command, "scan" | "hash" | "plan-import" | "materialize")
}

fn has_explicit_source_option(args: &[OsString], start: usize) -> bool {
    args.iter().skip(start).any(|arg| {
        let token = arg.to_string_lossy();
        token == "--source" || token.starts_with("--source=")
    })
}

fn find_positional_source_index(args: &[OsString], start: usize) -> Option<usize> {
    let mut idx = start;
    while idx < args.len() {
        let token = args[idx].to_string_lossy();
        if token == "--" {
            return (idx + 1 < args.len()).then_some(idx + 1);
        }
        if token.starts_with('-') {
            idx += 1;
            continue;
        }
        return Some(idx);
    }
    None
}
