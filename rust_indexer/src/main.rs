mod clean;
mod cleanup;
mod cli;
mod config;
mod db;
mod hash;
mod materialize;
mod model;
mod plan;
mod replay;
mod scan;
mod source;

use crate::model::IndexSource;
use cli::{Cli, Commands, ReportCommands, SourceCommands, SourceKind};

fn main() {
    if let Err(err) = run() {
        eprintln!("{err}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let cli = Cli::parse_with_source_aliases();
    config::initialize_indexer_settings()?;
    let repo = db::open_repo(cli.conninfo.as_deref())?;

    match cli.command {
        Commands::Source { command } => match command {
            SourceCommands::Add { name, path, kind } => {
                let source = scan::register_source(&repo, name.as_deref(), &path, kind)?;
                println!(
                    "Registered source {} as {} at {} (id={})",
                    source.name,
                    source.kind,
                    source.root_path.display(),
                    source.id_source
                );
                Ok(())
            }
            SourceCommands::List { kind, path } => match path {
                Some(path) => {
                    let (root_path, entries) = scan::list_source_directories(&repo, &path)?;
                    println!("FOD indexer source list");
                    println!("mode: browse");
                    println!("root: {}", root_path.display());
                    println!(
                        "kind hint: {}",
                        kind.as_ref().map(|kind| kind.as_str()).unwrap_or("none")
                    );
                    println!("directories: {}", entries.len());
                    for entry in entries {
                        if entry.added_sources.is_empty() {
                            let add_command = match kind.as_ref() {
                                Some(kind) => format!(
                                    "fod-indexer source add --kind {} --path {}",
                                    kind.as_str(),
                                    entry.path.display()
                                ),
                                None => format!(
                                    "fod-indexer source add --path {}",
                                    entry.path.display()
                                ),
                            };
                            println!(
                                "- available path={} add={}",
                                entry.path.display(),
                                add_command
                            );
                        } else {
                            println!(
                                "- added path={} sources={}",
                                entry.path.display(),
                                format_registered_sources(&entry.added_sources)
                            );
                        }
                    }
                    Ok(())
                }
                None => {
                    if matches!(kind, Some(SourceKind::Adb)) {
                        let adb_root = crate::source::adb_browse_root()?;
                        let (root_path, entries) =
                            scan::list_source_directories(&repo, &adb_root.local_root)?;
                        println!("FOD indexer source list");
                        println!("mode: adb-shell");
                        println!("device: {}", adb_root.serial);
                        println!("adb root: {}", adb_root.remote_root);
                        println!("root: {}", root_path.display());
                        println!("kind hint: adb");
                        println!("directories: {}", entries.len());
                        for entry in entries {
                            if entry.added_sources.is_empty() {
                                println!(
                                    "- available path={} add=fod-indexer source add --kind adb --path {}",
                                    entry.path.display(),
                                    entry.path.display()
                                );
                            } else {
                                println!(
                                    "- added path={} sources={}",
                                    entry.path.display(),
                                    format_registered_sources(&entry.added_sources)
                                );
                            }
                        }
                        Ok(())
                    } else {
                        let kind_filter = kind.as_ref().map(|kind| kind.as_str());
                        let sources = scan::list_sources(&repo, kind_filter)?;
                        println!("FOD indexer source list");
                        println!("kind filter: {}", kind_filter.unwrap_or("all"));
                        println!("registered sources: {}", sources.len());
                        for source in sources {
                            println!(
                                "- id={} name={} kind={} path={}",
                                source.id_source,
                                source.name,
                                source.kind,
                                source.root_path.display()
                            );
                        }
                        Ok(())
                    }
                }
            },
            SourceCommands::Remove { name } => {
                let source = scan::remove_source(&repo, &name)?;
                println!(
                    "Removed source {} as {} at {} (id={})",
                    source.name,
                    source.kind,
                    source.root_path.display(),
                    source.id_source
                );
                Ok(())
            }
        },
        Commands::Scan { source } => {
            let summary = scan::scan_source(&repo, &source)?;
            println!("{}", summary.human_readable());
            Ok(())
        }
        Commands::Hash {
            source,
            candidates_only,
        } => {
            let summary = hash::hash_source(&repo, &source, candidates_only)?;
            println!("{}", summary.human_readable());
            Ok(())
        }
        Commands::Report { command } => match command {
            ReportCommands::Duplicates { limit } => {
                plan::report_duplicate_sets(&repo, limit)?;
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
            println!("{}", summary.human_readable());
            Ok(())
        }
        Commands::CleanupFailed { plan } => {
            let summary = cleanup::cleanup_failed_materialization(&repo, plan)?;
            println!("{}", summary.human_readable());
            Ok(())
        }
        Commands::Clean { source, dry_run } => {
            let summary = clean::clean_source(&repo, &source, dry_run)?;
            println!("{}", summary.human_readable());
            Ok(())
        }
        Commands::Materialize { source, dry_run } => {
            let summary = materialize::materialize_source(&repo, &source, dry_run)?;
            println!("{}", summary.human_readable());
            Ok(())
        }
    }
}

fn format_registered_sources(sources: &[IndexSource]) -> String {
    sources
        .iter()
        .map(|source| {
            format!(
                "{} (kind={}, id={}, path={})",
                source.name,
                source.kind,
                source.id_source,
                source.root_path.display()
            )
        })
        .collect::<Vec<_>>()
        .join(", ")
}
