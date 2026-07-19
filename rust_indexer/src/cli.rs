use crate::capabilities::SourceCapabilities;
use clap::{Parser, Subcommand, ValueEnum};
use fod_rust_runtime::FOD_VERSION_LABEL;
use std::ffi::OsString;

#[derive(Debug, Clone, Parser)]
#[command(
    name = "fod-indexer",
    version = FOD_VERSION_LABEL,
    about = "Index external files before importing them into FOD.",
    long_about = "Index external files before importing them into FOD.\n\nUse fod-indexer to inspect its machine-readable capabilities, query the read-only file catalogue, register a filesystem-backed source, scan it, hash candidates, report duplicates, build a dry-run import plan, materialize files into FOD, or clean up a failed materialization.",
    after_long_help = "Examples:\n  fod-indexer capabilities\n  fod-indexer --output json capabilities\n  fod-indexer file list --limit 100\n  fod-indexer file search report --extension pdf --limit 25\n  fod-indexer file show --id 42\n  fod-indexer source add --path ~/Documents --kind local\n  fod-indexer source add --name lt7300_Documents --path ~/Documents --kind local\n  fod-indexer source add --path /mnt/qnap/share --kind qnap\n  fod-indexer source add --path /run/user/1000/gvfs/smb-share:server=192.168.1.11,share=Documents --kind smb\n  fod-indexer source add --path /run/user/1000/adb/0123456789ABCDEF --kind adb\n  fod-indexer source add --path ~/src/github.com/owner/repo --kind github\n  fod-indexer source list --kind adb\n  fod-indexer source list --path /run/user/1000/adb/0123456789ABCDEF --kind adb\n  fod-indexer source remove --name lt7300_Documents\n  fod-indexer scan --source lt7300_Documents\n  fod-indexer hash --source lt7300_Documents --candidates-only\n  fod-indexer report duplicates\n  fod-indexer report duplicates --id 7\n  fod-indexer plan-import --source lt7300_Documents --dry-run\n  fod-indexer plan list --limit 100\n  fod-indexer plan show --id 42\n  fod-indexer clean --source lt7300_Documents --dry-run\n  fod-indexer clean --source lt7300_Documents\n  fod-indexer materialize --source lt7300_Documents --dry-run\n  fod-indexer materialize --source lt7300_Documents\n  fod-indexer cleanup-failed --plan 42\n  fod-indexer --output json source list --kind adb"
)]
pub struct Cli {
    #[arg(long)]
    pub conninfo: Option<String>,
    #[arg(long, value_enum, default_value_t = OutputFormat::Text, help = "Select text or machine-readable JSON output.")]
    pub output: OutputFormat,
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
        about = "Describe the fod-indexer integration contract.",
        long_about = "Print the versioned fod-indexer capability document.\n\nThe command is read-only, does not require PostgreSQL, and distinguishes currently available read-only commands from commands that rebuild derived state and from planned read-only APIs. JSON output includes the stable API schema version and producer version."
    )]
    Capabilities,
    #[command(
        about = "Query indexed file records.",
        long_about = "List, search, or show the current read-only index file catalogue.\n\nThese commands query existing PostgreSQL index rows only. They do not scan sources, hash files, rebuild duplicate sets, create plans, or materialize data. Results use stable file ids and deterministic keyset pagination over a live catalogue view."
    )]
    File {
        #[command(subcommand)]
        command: FileCommands,
    },
    #[command(
        about = "Manage sources.",
        long_about = "Register, browse, list, or remove source adapters so fod-indexer can inspect roots before scan and materialize steps.\n\nThe current implementation keeps all supported source kinds on the shared path-backed flow and exposes their policy and capability profile explicitly, so future direct crawlers can be added without changing the basic registration contract. If --name is omitted, fod-indexer uses a kind-aware naming heuristic with the current hostname as the final fallback. Use --name to override that suggestion. Registered sources are stored with their kind, policy, capability profile, and canonical root path in PostgreSQL."
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
        long_about = "Show the duplicate groups discovered from the current hash state.\n\nThe report is built from the deduplicated hash tables and focuses on confirmed duplicate sets. Zero-size duplicate groups are skipped so cache and lock noise do not dominate the report; they remain in the hash tables and import planning. The no-id form rebuilds derived duplicate metadata before returning; use --id for a read-only lookup of an existing set."
    )]
    Report {
        #[command(subcommand)]
        command: ReportCommands,
    },
    #[command(
        about = "Inspect stored import plans.",
        long_about = "List or inspect stored import plans without rerunning the pipeline.\n\nUse plan list for deterministic read-only pagination over existing plans or plan show --id <id> to export one recorded plan snapshot. Neither command runs scan, hash, or import planning."
    )]
    Plan {
        #[command(subcommand)]
        command: PlanCommands,
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
        long_about = "Remove a failed materialization root and preserve shared data objects that are still referenced outside the failed tree.\n\nPass the plan id from the failed materialization run with --plan. Use this when materialize's automatic rollback could not finish or when you want to re-run the cleanup manually."
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
        long_about = "Validate a registered source and materialize its files into FOD.\n\nUse --dry-run to preview the current indexed state without writing PostgreSQL rows or creating import data.\n\nWarning: the non-dry-run command revalidates file metadata and hashes before importing. If a source file has disappeared, changed, or cannot be read during validation, the run aborts before any imported data is created. If a later import step fails after the import root has been created, the command now attempts to roll the partial tree back automatically and leaves cleanup-failed as the manual fallback. The non-dry-run command also requires the request-token schema migration because it creates replay-safe scan runs and import plans."
    )]
    Materialize {
        #[arg(long)]
        source: String,
        #[arg(long, default_value_t = false)]
        dry_run: bool,
    },
}

#[derive(Debug, Clone, Subcommand)]
pub enum FileCommands {
    #[command(
        about = "List indexed files.",
        long_about = "List existing indexed file records in ascending file-id order.\n\nThe command is strictly read-only and uses keyset pagination. Optional exact filters limit the catalogue by source name, file kind, scan status, or hash status. Pass next_cursor as --cursor to continue."
    )]
    List {
        #[arg(long, default_value_t = 100)]
        limit: usize,
        #[arg(long)]
        cursor: Option<u64>,
        #[arg(long)]
        source: Option<String>,
        #[arg(long)]
        file_kind: Option<String>,
        #[arg(long)]
        scan_status: Option<String>,
        #[arg(long)]
        hash_status: Option<String>,
    },
    #[command(
        about = "Search indexed files.",
        long_about = "Search existing indexed file records without scanning or modifying sources.\n\nThe optional positional QUERY searches path and source name. Additional filters cover path, basename, source, extension, file kind, scan status, hash status, size range, and modification-time range. At least one search filter is required. Results use file-id keyset pagination."
    )]
    Search {
        #[arg(help = "Case-insensitive text contained in indexed path or source name.")]
        query: Option<String>,
        #[arg(long)]
        path: Option<String>,
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        source: Option<String>,
        #[arg(long)]
        extension: Option<String>,
        #[arg(long)]
        file_kind: Option<String>,
        #[arg(long)]
        scan_status: Option<String>,
        #[arg(long)]
        hash_status: Option<String>,
        #[arg(long)]
        min_size: Option<u64>,
        #[arg(long)]
        max_size: Option<u64>,
        #[arg(long)]
        mtime_from: Option<i64>,
        #[arg(long)]
        mtime_to: Option<i64>,
        #[arg(long, default_value_t = 100)]
        limit: usize,
        #[arg(long)]
        cursor: Option<u64>,
    },
    #[command(
        about = "Show one indexed file.",
        long_about = "Show one existing indexed file record by its stable file id.\n\nThe command joins source and optional hash metadata but does not read file content or modify index state."
    )]
    Show {
        #[arg(long)]
        id: u64,
    },
}

#[derive(Debug, Clone, Subcommand)]
pub enum SourceCommands {
    #[command(
        about = "Add a source.",
        long_about = "Register a source adapter under a source name so scan, hash, plan-import, and materialize can use it later.\n\nChoose the adapter kind with --kind. Supported kinds are local, smb, qnap, adb, and github. The current implementation still reads a path-backed source root for all supported kinds, but the policy already distinguishes path-backed, mirrored, and export-backed flows so the direct-crawler decision stays explicit.\n\nIf --name is omitted, fod-indexer uses a kind-aware naming heuristic with the current hostname as the final fallback. Use --name to override that suggestion.",
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
        long_about = "Show registered source adapters and their canonical root paths, or browse a filesystem root and list its child directories.\n\nThe listing surfaces the explicit policy and capability profile for each source kind so path-backed and future crawler cases stay visible in the CLI. Use --kind adb without --path to probe the device through `adb shell`, discover its browsable storage root, and translate that root to a local gvfs mount when available so the printed add paths stay usable with --path. Use --path <PATH> to browse any explicit root and print child directories, with already registered entries marked as added. For other kinds with no --path, the command keeps listing registered sources.",
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
        long_about = "Print the duplicate groups currently known to the indexer.\n\nUse --id <id> to export an already stored duplicate-set snapshot without rebuilding the live duplicate tables. Without --id, the command refreshes derived duplicate metadata before producing the report and is not part of the strictly read-only API."
    )]
    Duplicates {
        #[arg(long, default_value_t = 100)]
        limit: usize,
        #[arg(long)]
        id: Option<u64>,
    },
}

#[derive(Debug, Clone, Subcommand)]
pub enum PlanCommands {
    #[command(
        about = "List stored import plans.",
        long_about = "List existing import plans without creating, refreshing, or modifying them.\n\nResults are ordered by plan id descending. Pass the returned next_cursor as --cursor to continue. --status applies an exact status filter. --limit must be between 1 and 1000."
    )]
    List {
        #[arg(long, default_value_t = 100)]
        limit: usize,
        #[arg(long)]
        cursor: Option<u64>,
        #[arg(long)]
        status: Option<String>,
    },
    #[command(
        about = "Show a stored import plan.",
        long_about = "Export a stored import plan by id.\n\nThis command reads the plan snapshot that was already written earlier and does not rerun scan, hash, or import planning."
    )]
    Show {
        #[arg(long)]
        id: u64,
    },
}

#[derive(Debug, Copy, Clone, Eq, PartialEq, ValueEnum)]
pub enum OutputFormat {
    Text,
    Json,
}

impl OutputFormat {
    pub fn is_json(self) -> bool {
        matches!(self, OutputFormat::Json)
    }
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
            "--output" => {
                idx = idx.saturating_add(2);
            }
            "-h" | "--help" | "-V" | "--version" => {
                idx += 1;
            }
            "scan" | "hash" | "plan-import" | "clean" | "materialize" => return Some(idx),
            "capabilities" | "file" | "source" | "report" | "plan" | "cleanup-failed" => {
                return None
            }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_offline_capabilities_with_json_output() {
        let cli = Cli::try_parse_from(["fod-indexer", "--output", "json", "capabilities"])
            .expect("capabilities command should parse");
        assert_eq!(cli.output, OutputFormat::Json);
        assert!(matches!(cli.command, Commands::Capabilities));
    }

    #[test]
    fn parses_plan_list_filters() {
        let cli = Cli::try_parse_from([
            "fod-indexer",
            "--output",
            "json",
            "plan",
            "list",
            "--limit",
            "25",
            "--cursor",
            "42",
            "--status",
            "dry_run_completed",
        ])
        .expect("plan list command should parse");
        assert_eq!(cli.output, OutputFormat::Json);
        match cli.command {
            Commands::Plan {
                command:
                    PlanCommands::List {
                        limit,
                        cursor,
                        status,
                    },
            } => {
                assert_eq!(limit, 25);
                assert_eq!(cursor, Some(42));
                assert_eq!(status.as_deref(), Some("dry_run_completed"));
            }
            _ => panic!("expected plan list command"),
        }
    }

    #[test]
    fn parses_file_list_filters() {
        let cli = Cli::try_parse_from([
            "fod-indexer",
            "file",
            "list",
            "--limit",
            "25",
            "--cursor",
            "42",
            "--source",
            "lt7300_Documents",
            "--scan-status",
            "ok",
        ])
        .expect("file list command should parse");
        match cli.command {
            Commands::File {
                command:
                    FileCommands::List {
                        limit,
                        cursor,
                        source,
                        scan_status,
                        ..
                    },
            } => {
                assert_eq!(limit, 25);
                assert_eq!(cursor, Some(42));
                assert_eq!(source.as_deref(), Some("lt7300_Documents"));
                assert_eq!(scan_status.as_deref(), Some("ok"));
            }
            _ => panic!("expected file list command"),
        }
    }

    #[test]
    fn parses_file_search_filters() {
        let cli = Cli::try_parse_from([
            "fod-indexer",
            "--output",
            "json",
            "file",
            "search",
            "report",
            "--extension",
            "pdf",
            "--min-size",
            "100",
            "--max-size",
            "10000",
            "--limit",
            "10",
        ])
        .expect("file search command should parse");
        assert_eq!(cli.output, OutputFormat::Json);
        match cli.command {
            Commands::File {
                command:
                    FileCommands::Search {
                        query,
                        extension,
                        min_size,
                        max_size,
                        limit,
                        ..
                    },
            } => {
                assert_eq!(query.as_deref(), Some("report"));
                assert_eq!(extension.as_deref(), Some("pdf"));
                assert_eq!(min_size, Some(100));
                assert_eq!(max_size, Some(10_000));
                assert_eq!(limit, 10);
            }
            _ => panic!("expected file search command"),
        }
    }

    #[test]
    fn parses_file_show_id() {
        let cli = Cli::try_parse_from(["fod-indexer", "file", "show", "--id", "17"])
            .expect("file show command should parse");
        match cli.command {
            Commands::File {
                command: FileCommands::Show { id },
            } => assert_eq!(id, 17),
            _ => panic!("expected file show command"),
        }
    }

    #[test]
    fn capabilities_is_not_treated_as_a_positional_source_command() {
        let args = normalize_indexer_args([
            OsString::from("fod-indexer"),
            OsString::from("capabilities"),
        ]);
        assert_eq!(
            args,
            vec![
                OsString::from("fod-indexer"),
                OsString::from("capabilities")
            ]
        );
    }
}
