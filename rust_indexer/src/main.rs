mod cli;
mod db;
mod hash;
mod materialize;
mod model;
mod plan;
mod scan;

use clap::Parser;
use cli::{Cli, Commands, ReportCommands, SourceCommands};

fn main() {
    if let Err(err) = run() {
        eprintln!("{err}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let cli = Cli::parse();
    let repo = db::open_repo(cli.conninfo.as_deref())?;

    match cli.command {
        Commands::Source { command } => match command {
            SourceCommands::Add { name, path, kind } => {
                let source = scan::register_source(&repo, &name, &path, kind.as_str())?;
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
        Commands::PlanImport { dry_run } => {
            if !dry_run {
                return Err("Only --dry-run is supported for now.".to_string());
            }
            let summary = plan::dry_run_import_plan(&repo)?;
            println!("{}", summary.human_readable());
            Ok(())
        }
        Commands::Materialize { source } => {
            let summary = materialize::materialize_source(&repo, &source)?;
            println!("{}", summary.human_readable());
            Ok(())
        }
    }
}
