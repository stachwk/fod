mod clean;
mod cleanup;
mod cli;
mod db;
mod hash;
mod materialize;
mod model;
mod plan;
mod replay;
mod scan;
mod source;

use cli::{Cli, Commands, ReportCommands, SourceCommands};

fn main() {
    if let Err(err) = run() {
        eprintln!("{err}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let cli = Cli::parse_with_source_aliases();
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
