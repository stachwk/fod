use crate::capabilities::SourceCapabilities;
use clap::{Parser, Subcommand, ValueEnum};
use fod_rust_runtime::FOD_VERSION_LABEL;
use std::ffi::OsString;

#[derive(Debug, Clone, Parser)]
#[command(
    name = "fod-indexer",
    version = FOD_VERSION_LABEL,
    about = "Index external files before importing them into FOD.",
    long_about = "Index external files before importing them into FOD.\n\nUse fod-indexer to register a filesystem-backed source, scan it, hash candidates, report duplicates, build a dry-run import plan, materialize files into FOD, or clean up a failed materialization.",
    after_long_help = "Examples:\n  fod-indexer source add --path ~/Documents --kind local\n  fod-indexer source add --name lt7300_Documents --path ~/Documents --kind local\n  fod-indexer source add --path /mnt/qnap/share --kind qnap\n  fod-indexer source add --path /run/user/1000/gvfs/smb-share:server=192.168.1.11,share=Documents --kind smb\n  fod-indexer source add --path /run/user/1000/adb/0123456789ABCDEF --kind adb\n  fod-indexer source add --path ~/src/github.com/owner/repo --kind github\n  fod-indexer source list --kind adb\n  fod-indexer source list --path /run/user/1000/adb/0123456789ABCDEF --kind adb\n  fod-indexer source remove --name lt7300_Documents\n  fod-indexer scan --source lt7300_Documents\n  fod-indexer hash --source lt7300_Documents --candidates-only\n  fod-indexer report duplicates\n  fod-indexer plan-import --source lt7300_Documents --dry-run\n  fod-indexer clean --source lt7300_Documents --dry-run\n  fod-indexer clean --source lt7300_Documents\n  fod-indexer materialize --source lt7300_Documents --dry-run\n  fod-indexer materialize --source lt7300_Documents\n  fod-indexer cleanup-failed --plan 42"
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
        about = "Manage sources.",
        long_about = "Register, browse, list, or remove source adapters so fod-indexer can inspect roots before scan and materialize steps.\n\nThe current implementation keeps all supported source kinds on the shared path-backed flow and exposes their capability profile explicitly, so future direct crawlers can be added without changing the basic registration contract. If --name is omitted, fod-indexer uses a kind-aware naming heuristic with the current hostname as the final fallback. Use --name to override that suggestion. Registered sources are stored with their kind, capability profile, and canonical root path in PostgreSQL."
    )]
    Source {
        #[command(subcommand)]
        command: SourceCommands,
    },
    #[command(
        about = "Scan a source directory.",
        long_about = "Walk the registered source directory and store file metadata in index_files.\n\nThe scan records regular, unreadable, and unsupported entries before hashing. Zero-length files are skipped before they reach the index so they do not enter hashing, planning, or materialization. The command also emits periodic progress lines on stderr while it walks the tree. It needs the request-token schema migration because it creates replay-safe scan runs."
    )]
    Scan {
        #[arg(long)]
        source: String,
    },
    #[command(
        about = "Hash scanned files.",
        long_about = "Compute partial and full hashes for candidate files in a registered source.\n\nUse --candidates-only to skip files that are not plausible duplicate candidates by size. The command also emits periodic progress lines on stderr while it hashes files and rebuilds duplicate sets."
    )]
    Hash {
        #[arg(long)]
        source: String,
        #[arg(long, default_value_t = false)]
        candidates_only: bool,
    },
    #[command(
        about = "Report duplicate groups.",
        long_about = "Show the duplicate groups discovered from the current hash state.\n\nThe report is built from the deduplicated hash tables and focuses on confirmed duplicate sets. Zero-size duplicate groups are skipped so cache and lock noise do not dominate the report; they remain in the hash tables and import planning."
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
        about = "Clean stale index entries for a source.",
        long_about = "Compare the current source tree with the indexed rows for a source and remove file entries that no longer exist or should now be ignored.\n\nUse --dry-run to preview which rows would be removed without touching PostgreSQL. A real cleanup also refreshes duplicate-set metadata after pruning stale rows."
    )]
    Clean {
        #[arg(long)]
        source: String,
        #[arg(long, default_value_t = false)]
        dry_run: bool,
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
        about = "Add a source.",
        long_about = "Register a source adapter under a source name so scan, hash, plan-import, and materialize can use it later.\n\nChoose the adapter kind with --kind. Supported kinds are local, smb, qnap, adb, and github. The current implementation still reads a path-backed source root for all supported kinds, but the capability profile already distinguishes path-backed, mirrored, and future direct-crawler cases.\n\nIf --name is omitted, fod-indexer uses a kind-aware naming heuristic with the current hostname as the final fallback. Use --name to override that suggestion.",
        override_usage = "fod-indexer source add --path <PATH> [--name <NAME>] [--kind <KIND>]"
    )]
    Add {
        #[arg(
            long,
            help = "Optional explicit source name. If omitted, fod-indexer picks a kind-aware default."
        )]
        name: Option<String>,
        #[arg(
            long,
            help = "Filesystem path for the source root. For adb or github kinds, this is still a local path-backed root for now."
        )]
        path: String,
        #[arg(
            long,
            value_enum,
            default_value_t = SourceKind::Local,
            help = "Select the adapter kind: local, smb, qnap, adb, or github."
        )]
        kind: SourceKind,
    },
    #[command(
        about = "List registered sources or browse a filesystem root.",
        long_about = "Show registered source adapters and their canonical root paths, or browse a filesystem root and list its child directories.\n\nThe listing surfaces the explicit capability profile for each source kind so path-backed and future crawler cases stay visible in the CLI. Use --kind adb without --path to probe the device through `adb shell`, discover its browsable storage root, and translate that root to a local gvfs mount when available so the printed add paths stay usable with --path. Use --path <PATH> to browse any explicit root and print child directories, with already registered entries marked as added. For other kinds with no --path, the command keeps listing registered sources.",
        override_usage = "fod-indexer source list [--kind <KIND>] [--path <PATH>]"
    )]
    List {
        #[arg(
            long,
            value_enum,
            help = "Filter the registered-source listing to a single adapter kind, or use it as the suggested kind when browsing with --path."
        )]
        kind: Option<SourceKind>,
        #[arg(
            long,
            help = "Browse this filesystem root instead of listing registered sources."
        )]
        path: Option<String>,
    },
    #[command(
        about = "Remove a registered source.",
        long_about = "Remove a source registration from PostgreSQL. The indexed rows for that source are removed through foreign-key cascade."
    )]
    Remove {
        #[arg(
            long,
            help = "Remove the source by its registered name. The name is unique in the source table."
        )]
        name: String,
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
    Smb,
    Qnap,
    Adb,
    Github,
}

impl SourceKind {
    pub fn as_str(self) -> &'static str {
        match self {
            SourceKind::Local => "local",
            SourceKind::Smb => "smb",
            SourceKind::Qnap => "qnap",
            SourceKind::Adb => "adb",
            SourceKind::Github => "github",
        }
    }

    pub fn from_db_str(value: &str) -> Option<Self> {
        match value {
            "local" => Some(SourceKind::Local),
            "smb" => Some(SourceKind::Smb),
            "qnap" => Some(SourceKind::Qnap),
            "adb" => Some(SourceKind::Adb),
            "github" => Some(SourceKind::Github),
            _ => None,
        }
    }

    pub const fn capabilities(self) -> SourceCapabilities {
        match self {
            SourceKind::Local => SourceCapabilities::new(true, true, false, false, false),
            SourceKind::Smb => SourceCapabilities::new(true, true, true, false, true),
            SourceKind::Qnap => SourceCapabilities::new(true, true, true, false, true),
            SourceKind::Adb => SourceCapabilities::new(true, true, true, true, true),
            SourceKind::Github => SourceCapabilities::new(true, true, true, true, true),
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
            "scan" | "hash" | "plan-import" | "clean" | "materialize" => return Some(idx),
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
    matches!(
        command,
        "scan" | "hash" | "plan-import" | "clean" | "materialize"
    )
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
