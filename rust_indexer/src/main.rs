mod capabilities;
mod clean;
mod cleanup;
mod cli;
mod config;
mod db;
mod duplicate_set_api;
mod file_read_api;
mod hash;
mod materialize;
mod model;
mod output;
mod plan;
mod progress;
mod read_api;
mod scan;
mod source;
mod source_registry;

use crate::model::IndexSource;
use cli::{
    Cli, Commands, DuplicateSetCommands, FileCommands, PlanCommands, ReportCommands,
    SourceCommands, SourceKind,
};
use output::{print_json, SourceListOutput, SourceMutationOutput};
use std::io::Write;
use std::path::Path;

fn main() {
    if let Err(err) = run() {
        eprintln!("{err}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let cli = Cli::parse_with_source_aliases();
    let output = cli.output;

    if matches!(&cli.command, Commands::Capabilities) {
        let capabilities = file_read_api::capabilities_output();
        if output.is_json() {
            print_json(&capabilities)?;
        } else {
            println!("{}", capabilities.human_readable());
        }
        return Ok(());
    }

    config::initialize_indexer_settings()?;
    let repo = db::open_repo(cli.conninfo.as_deref())?;

    match cli.command {
        Commands::Capabilities => unreachable!("capabilities returns before opening PostgreSQL"),
        Commands::DuplicateSet { command } => match command {
            DuplicateSetCommands::List { limit, cursor } => {
                let sets = duplicate_set_api::load_duplicate_set_list(&repo, limit, cursor)?;
                if output.is_json() {
                    print_json(&sets)?;
                } else {
                    println!("{}", sets.human_readable());
                }
                Ok(())
            }
        },
        Commands::File { command } => match command {
            FileCommands::List {
                limit,
                cursor,
                source,
                file_kind,
                scan_status,
                hash_status,
            } => {
                let files = read_api::load_file_list(
                    &repo,
                    limit,
                    cursor,
                    source.as_deref(),
                    file_kind.as_deref(),
                    scan_status.as_deref(),
                    hash_status.as_deref(),
                )?;
                if output.is_json() {
                    print_json(&files)?;
                } else {
                    println!("{}", files.human_readable());
                }
                Ok(())
            }
            FileCommands::Search {
                query,
                path,
                name,
                source,
                extension,
                file_kind,
                scan_status,
                hash_status,
                min_size,
                max_size,
                mtime_from,
                mtime_to,
                limit,
                cursor,
            } => {
                let files = read_api::search_files(
                    &repo,
                    limit,
                    cursor,
                    query.as_deref(),
                    path.as_deref(),
                    name.as_deref(),
                    source.as_deref(),
                    extension.as_deref(),
                    file_kind.as_deref(),
                    scan_status.as_deref(),
                    hash_status.as_deref(),
                    min_size,
                    max_size,
                    mtime_from,
                    mtime_to,
                )?;
                if output.is_json() {
                    print_json(&files)?;
                } else {
                    println!("{}", files.human_readable());
                }
                Ok(())
            }
            FileCommands::Show { id } => {
                let file = read_api::show_file(&repo, id)?;
                if output.is_json() {
                    print_json(&file)?;
                } else {
                    println!("{}", file.human_readable());
                }
                Ok(())
            }
            FileCommands::Read { id, offset, length } => {
                let file = file_read_api::read_file(&repo, id, offset, length)?;
                if output.is_json() {
                    print_json(&file)?;
                } else {
                    eprintln!("{}", file.provenance_human_readable());
                    let mut stdout = std::io::stdout().lock();
                    stdout
                        .write_all(file.bytes())
                        .map_err(|err| format!("file_read_io_error: stdout write failed: {err}"))?;
                    stdout
                        .flush()
                        .map_err(|err| format!("file_read_io_error: stdout flush failed: {err}"))?;
                }
                Ok(())
            }
        },
        Commands::Source { command } => match command {
            SourceCommands::Add { name, path, kind } => {
                let source = source_registry::register_source(&repo, name.as_deref(), &path, kind)?;
                if output.is_json() {
                    print_json(&SourceMutationOutput {
                        source: (&source).into(),
                    })?;
                } else {
                    println!(
                        "Registered source {} as {} at {} (id={})",
                        source.name,
                        source.kind,
                        source.root_path.display(),
                        source.id_source
                    );
                    println!("policy: {}", kind.capabilities().policy());
                    println!("capabilities: {}", kind.capabilities());
                }
                Ok(())
            }
            SourceCommands::List { kind, path } => match path {
                Some(path) => {
                    let (root_path, entries) =
                        source_registry::list_source_directories(&repo, &path)?;
                    if output.is_json() {
                        print_json(&SourceListOutput::browse(
                            kind.as_ref().map(|kind| kind.as_str().to_string()),
                            root_path.display().to_string(),
                            entries,
                            kind.as_ref().map(|kind| kind.capabilities().policy()),
                            kind.as_ref().map(|kind| kind.capabilities()),
                        ))?;
                    } else {
                        println!("FOD indexer source list");
                        println!("mode: browse");
                        println!("root: {}", root_path.display());
                        println!(
                            "kind hint: {}",
                            kind.as_ref().map(|kind| kind.as_str()).unwrap_or("none")
                        );
                        if let Some(kind) = kind.as_ref() {
                            println!("policy: {}", kind.capabilities().policy());
                            println!("capabilities: {}", kind.capabilities());
                        }
                        println!("directories: {}", entries.len());
                        for entry in entries {
                            if entry.added_sources.is_empty() {
                                println!("- available path={}", entry.path.display());
                                println!(
                                    "  {}",
                                    render_source_add_command(kind.as_ref(), &entry.path)
                                );
                            } else {
                                println!(
                                    "- added path={} sources={}",
                                    entry.path.display(),
                                    format_registered_sources(&entry.added_sources)
                                );
                            }
                        }
                    }
                    Ok(())
                }
                None => {
                    if matches!(kind, Some(SourceKind::Adb)) {
                        let adb_root = crate::source::adb_browse_root()?;
                        let (root_path, entries) =
                            source_registry::list_source_directories(&repo, &adb_root.local_root)?;
                        if output.is_json() {
                            print_json(&SourceListOutput::adb(
                                adb_root.serial,
                                adb_root.remote_root,
                                root_path.display().to_string(),
                                entries,
                            ))?;
                        } else {
                            println!("FOD indexer source list");
                            println!("mode: adb-shell");
                            println!("device: {}", adb_root.serial);
                            println!("adb root: {}", adb_root.remote_root);
                            println!("root: {}", root_path.display());
                            println!("kind hint: adb");
                            println!("policy: {}", SourceKind::Adb.capabilities().policy());
                            println!("capabilities: {}", SourceKind::Adb.capabilities());
                            println!("directories: {}", entries.len());
                            for entry in entries {
                                if entry.added_sources.is_empty() {
                                    println!("- available path={}", entry.path.display());
                                    println!(
                                        "  {}",
                                        render_source_add_command(
                                            Some(&SourceKind::Adb),
                                            &entry.path
                                        )
                                    );
                                } else {
                                    println!(
                                        "- added path={} sources={}",
                                        entry.path.display(),
                                        format_registered_sources(&entry.added_sources)
                                    );
                                }
                            }
                        }
                        Ok(())
                    } else {
                        let kind_filter = kind.as_ref().map(|kind| kind.as_str());
                        let sources = source_registry::list_sources(&repo, kind_filter)?;
                        if output.is_json() {
                            print_json(&SourceListOutput::registered(
                                kind_filter.map(|value| value.to_string()),
                                sources,
                            ))?;
                        } else {
                            println!("FOD indexer source list");
                            println!("kind filter: {}", kind_filter.unwrap_or("all"));
                            println!("registered sources: {}", sources.len());
                            for source in sources {
                                println!(
                                    "- id={} name={} kind={} policy={} capabilities={} path={}",
                                    source.id_source,
                                    source.name,
                                    source.kind,
                                    source_kind_policy(&source.kind),
                                    source_kind_capabilities(&source.kind),
                                    source.root_path.display()
                                );
                            }
                        }
                        Ok(())
                    }
                }
            },
            SourceCommands::Remove { name } => {
                let source = source_registry::remove_source(&repo, &name)?;
                if output.is_json() {
                    print_json(&SourceMutationOutput {
                        source: (&source).into(),
                    })?;
                } else {
                    println!(
                        "Removed source {} as {} at {} (id={})",
                        source.name,
                        source.kind,
                        source.root_path.display(),
                        source.id_source
                    );
                }
                Ok(())
            }
        },
        Commands::Scan { source } => {
            let summary = scan::scan_source(&repo, &source)?;
            if output.is_json() {
                print_json(&summary)?;
            } else {
                println!("{}", summary.human_readable());
            }
            Ok(())
        }
        Commands::Hash {
            source,
            candidates_only,
        } => {
            let summary = hash::hash_source(&repo, &source, candidates_only)?;
            if output.is_json() {
                print_json(&summary)?;
            } else {
                println!("{}", summary.human_readable());
            }
            Ok(())
        }
        Commands::Report { command } => match command {
            ReportCommands::Duplicates { limit, id } => {
                if let Some(id) = id {
                    let snapshot = plan::load_duplicate_set_snapshot(&repo, id)?;
                    if output.is_json() {
                        print_json(&snapshot)?;
                    } else {
                        println!("FOD indexer duplicate report");
                        println!("{}", snapshot.human_readable());
                    }
                } else if output.is_json() {
                    let snapshot = plan::load_duplicate_report_snapshot(&repo, limit)?;
                    print_json(&snapshot)?;
                } else {
                    plan::report_duplicate_sets(&repo, limit)?;
                }
                Ok(())
            }
        },
        Commands::Plan { command } => match command {
            PlanCommands::List {
                limit,
                cursor,
                status,
            } => {
                let plans =
                    read_api::load_import_plan_list(&repo, limit, cursor, status.as_deref())?;
                if output.is_json() {
                    print_json(&plans)?;
                } else {
                    println!("{}", plans.human_readable());
                }
                Ok(())
            }
            PlanCommands::Show { id } => {
                let snapshot = plan::load_import_plan_snapshot(&repo, id)?;
                if output.is_json() {
                    print_json(&snapshot)?;
                } else {
                    println!("{}", snapshot.human_readable());
                }
                Ok(())
            }
        },
        Commands::PlanImport {
            source,
            all_sources,
            dry_run,
        } => {
            if !dry_run {
                return Err("Only --dry-run is supported for now.".to_string());
            }
            let source_filter = match (source.as_deref(), all_sources) {
                (Some(_), true) => {
                    return Err("Use either --source <name> or --all-sources, not both.".to_string())
                }
                (Some(source), false) => Some(source),
                (None, true) => None,
                (None, false) => {
                    return Err(
                        "Specify --source <name> or --all-sources for plan-import.".to_string()
                    )
                }
            };
            let summary = plan::dry_run_import_plan(&repo, source_filter)?;
            if output.is_json() {
                print_json(&summary)?;
            } else {
                println!("{}", summary.human_readable());
            }
            Ok(())
        }
        Commands::CleanupFailed { plan } => {
            let summary = cleanup::cleanup_failed_materialization(&repo, plan)?;
            if output.is_json() {
                print_json(&summary)?;
            } else {
                println!("{}", summary.human_readable());
            }
            Ok(())
        }
        Commands::Clean { source, dry_run } => {
            let summary = clean::clean_source(&repo, &source, dry_run)?;
            if output.is_json() {
                print_json(&summary)?;
            } else {
                println!("{}", summary.human_readable());
            }
            Ok(())
        }
        Commands::Materialize { source, dry_run } => {
            let summary = materialize::materialize_source(&repo, &source, dry_run)?;
            if output.is_json() {
                print_json(&summary)?;
            } else {
                println!("{}", summary.human_readable());
            }
            Ok(())
        }
    }
}

fn render_source_add_command(kind: Option<&SourceKind>, path: &Path) -> String {
    let path_text = path.display().to_string();
    let quoted_path = shell_quote(&path_text);
    match kind {
        Some(kind) => format!(
            "fod-indexer source add --kind {} --path {}",
            kind.as_str(),
            quoted_path
        ),
        None => format!("fod-indexer source add --path {}", quoted_path),
    }
}

fn shell_quote(value: &str) -> String {
    if value.is_empty() {
        return "''".to_string();
    }

    let mut quoted = String::with_capacity(value.len() + 2);
    quoted.push('\'');
    for ch in value.chars() {
        if ch == '\'' {
            quoted.push_str("'\\''");
        } else {
            quoted.push(ch);
        }
    }
    quoted.push('\'');
    quoted
}

fn format_registered_sources(sources: &[IndexSource]) -> String {
    sources
        .iter()
        .map(|source| {
            format!(
                "{} (kind={}, policy={}, capabilities={}, id={}, path={})",
                source.name,
                source.kind,
                source_kind_policy(&source.kind),
                source_kind_capabilities(&source.kind),
                source.id_source,
                source.root_path.display()
            )
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn source_kind_capabilities(kind: &str) -> String {
    SourceKind::from_db_str(kind)
        .map(|kind| kind.capabilities().to_string())
        .unwrap_or_else(|| "unavailable".to_string())
}

fn source_kind_policy(kind: &str) -> String {
    SourceKind::from_db_str(kind)
        .map(|kind| kind.capabilities().policy().to_string())
        .unwrap_or_else(|| "unavailable".to_string())
}
