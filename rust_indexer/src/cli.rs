use clap::{Parser, Subcommand, ValueEnum};
use fod_rust_runtime::FOD_VERSION_LABEL;
use std::ffi::OsString;

#[derive(Debug, Clone, Parser)]
#[command(
    name = "fod-indexer",
    version = FOD_VERSION_LABEL,
    about = "Index external files before importing them into FOD.",
    long_about = "Index external files before importing them into FOD.\n\nUse fod-indexer to register a local source, scan it, hash candidates, report duplicates, build a dry-run import plan, materialize files into FOD, or clean up a failed materialization.",
    after_long_help = "Examples:\n  fod-indexer source add --path ~/Documents --kind local\n  fod-indexer source add --name lt7300_Documents --path ~/Documents --kind local\n  fod-indexer scan --source lt7300_Documents\n  fod-indexer hash --source lt7300_Documents --candidates-only\n  fod-indexer report duplicates\n  fod-indexer plan-import --source lt7300_Documents --dry-run\n  fod-indexer materialize --source lt7300_Documents --dry-run\n  fod-indexer materialize --source lt7300_Documents\n  fod-indexer cleanup-failed --plan 42"
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
    #[command(
        about = "Register a source directory.",
        long_about = "Register a local directory so fod-indexer can scan and materialize it later.\n\nIf --name is omitted, fod-indexer uses the current hostname as the default local source name. Use --name to override that suggestion. This command stores the source name, kind, and canonical root path in PostgreSQL."
    )]
    Source {
        #[command(subcommand)]
        command: SourceCommands,
    },
    #[command(
        about = "Scan a source directory.",
        long_about = "Walk the registered source directory and store file metadata in index_files.\n\nThe scan records regular, unreadable, and unsupported entries before hashing. It also needs the request-token schema migration because it creates replay-safe scan runs."
    )]
    Scan {
        #[arg(long)]
        source: String,
    },
    #[command(
        about = "Hash scanned files.",
        long_about = "Compute partial and full hashes for candidate files in a registered source.\n\nUse --candidates-only to skip files that are not plausible duplicate candidates by size."
    )]
    Hash {
        #[arg(long)]
        source: String,
        #[arg(long, default_value_t = false)]
        candidates_only: bool,
    },
    #[command(
        about = "Report duplicate groups.",
        long_about = "Show the duplicate groups discovered from the current hash state.\n\nThe report is built from the deduplicated hash tables and focuses on confirmed duplicate sets."
    )]
    Report {
        #[command(subcommand)]
        command: ReportCommands,
    },
    #[command(
        about = "Create a dry-run import plan.",
        long_about = "Build a dry-run import plan for one source or for all sources.\n\nUse --source <name> for a single registered source or --all-sources for the global view. This command requires the request-token schema migration because it creates replay-safe import plans."
    )]
    PlanImport {
        #[arg(long)]
        source: Option<String>,
        #[arg(long, default_value_t = false)]
        all_sources: bool,
        #[arg(long, default_value_t = false)]
        dry_run: bool,
    },
    #[command(
        about = "Clean up a failed materialization.",
        long_about = "Remove a failed materialization root and preserve shared data objects that are still referenced outside the failed tree.\n\nPass the plan id from the failed materialization run with --plan."
    )]
    CleanupFailed {
        #[arg(long)]
        plan: u64,
    },
    #[command(
        about = "Materialize a source into FOD.",
        long_about = "Validate a registered source and materialize its files into FOD.\n\nUse --dry-run to preview the current indexed state without writing PostgreSQL rows or creating import data.\n\nWarning: the non-dry-run command revalidates file metadata and hashes before importing. If a source file has disappeared, changed, or cannot be read during validation, the run aborts before any imported data is created. The non-dry-run command also requires the request-token schema migration because it creates replay-safe scan runs and import plans."
    )]
    Materialize {
        #[arg(long)]
        source: String,
        #[arg(long, default_value_t = false)]
        dry_run: bool,
    },
}

#[derive(Debug, Clone, Subcommand)]
pub enum SourceCommands {
    #[command(
        about = "Add a local source.",
        long_about = "Register a local directory under a source name so scan, hash, plan-import, and materialize can use it later."
    )]
    Add {
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        path: String,
        #[arg(long, value_enum, default_value_t = SourceKind::Local)]
        kind: SourceKind,
    },
}

#[derive(Debug, Clone, Subcommand)]
pub enum ReportCommands {
    #[command(
        about = "Show duplicate files.",
        long_about = "Print the duplicate groups currently known to the indexer."
    )]
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
