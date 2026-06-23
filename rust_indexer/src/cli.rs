use clap::{Parser, Subcommand, ValueEnum};
use fod_rust_runtime::FOD_VERSION_LABEL;

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
        #[arg(long, default_value_t = false)]
        dry_run: bool,
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
