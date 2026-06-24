use crate::cli::SourceKind;
use crate::db::sql_quote_literal;
use crate::model::{IndexSource, SourceBrowseEntry};
use crate::source;
use fod_rust_hotpath::pg::DbRepo;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

pub fn register_source(
    repo: &DbRepo,
    name: Option<&str>,
    path: &str,
    kind: SourceKind,
) -> Result<IndexSource, String> {
    let root_path = fs::canonicalize(path)
        .map_err(|err| format!("source path {path} is not accessible: {err}"))?;
    let name = source::resolve_source_name(name, kind, &root_path)?;
    let metadata = fs::metadata(&root_path)
        .map_err(|err| format!("source path {} is not readable: {err}", root_path.display()))?;
    if !metadata.is_dir() {
        return Err(format!(
            "source path {} is not a directory",
            root_path.display()
        ));
    }

    let sql = format!(
        "
        INSERT INTO index_sources (name, kind, root_path, created_at, updated_at)
        VALUES ({name}, {kind}, {root_path}, NOW(), NOW())
        ON CONFLICT (name) DO UPDATE SET
            kind = EXCLUDED.kind,
            root_path = EXCLUDED.root_path,
            updated_at = NOW()
        RETURNING id_index_source, name, kind, root_path
        ",
        name = sql_quote_literal(&name),
        kind = sql_quote_literal(kind.as_str()),
        root_path = sql_quote_literal(&root_path.to_string_lossy()),
    );
    let rows = repo.query_rows_text(&sql)?;
    let row = rows
        .first()
        .ok_or_else(|| "source registration did not return a row".to_string())?;
    IndexSource::from_row(row)
}

pub fn load_source(repo: &DbRepo, name: &str) -> Result<IndexSource, String> {
    let sql = format!(
        "
        SELECT id_index_source, name, kind, root_path
        FROM index_sources
        WHERE name = {}
        LIMIT 1
        ",
        sql_quote_literal(name)
    );
    let rows = repo.query_rows_text(&sql)?;
    let row = rows
        .first()
        .ok_or_else(|| format!("unknown source: {name}"))?;
    IndexSource::from_row(row)
}

pub fn list_sources(repo: &DbRepo, kind_filter: Option<&str>) -> Result<Vec<IndexSource>, String> {
    let where_clause = kind_filter
        .map(|kind| format!("WHERE kind = {}", sql_quote_literal(kind)))
        .unwrap_or_default();
    let sql = format!(
        "
        SELECT id_index_source, name, kind, root_path
        FROM index_sources
        {where_clause}
        ORDER BY kind, name, id_index_source
        ",
    );
    let rows = repo.query_rows_text(&sql)?;
    rows.iter().map(|row| IndexSource::from_row(row)).collect()
}

pub fn remove_source(repo: &DbRepo, name: &str) -> Result<IndexSource, String> {
    let sql = format!(
        "
        DELETE FROM index_sources
        WHERE name = {}
        RETURNING id_index_source, name, kind, root_path
        ",
        sql_quote_literal(name)
    );
    let rows = repo.query_rows_text(&sql)?;
    let row = rows
        .first()
        .ok_or_else(|| format!("unknown source: {name}"))?;
    IndexSource::from_row(row)
}

pub fn list_source_directories<P: AsRef<Path>>(
    repo: &DbRepo,
    path: P,
) -> Result<(PathBuf, Vec<SourceBrowseEntry>), String> {
    let path = path.as_ref();
    let root_path = fs::canonicalize(path)
        .map_err(|err| format!("source path {} is not accessible: {err}", path.display()))?;
    let metadata = fs::metadata(&root_path)
        .map_err(|err| format!("source path {} is not readable: {err}", root_path.display()))?;
    if !metadata.is_dir() {
        return Err(format!(
            "source path {} is not a directory",
            root_path.display()
        ));
    }

    let registered_sources = list_sources(repo, None)?;
    let mut registered_by_root: HashMap<PathBuf, Vec<IndexSource>> = HashMap::new();
    for source in registered_sources {
        registered_by_root
            .entry(source.root_path.clone())
            .or_default()
            .push(source);
    }

    let mut entries = Vec::new();
    let read_dir = fs::read_dir(&root_path)
        .map_err(|err| format!("source path {} is not readable: {err}", root_path.display()))?;
    for item in read_dir {
        let entry = match item {
            Ok(entry) => entry,
            Err(err) => {
                eprintln!("FOD indexer source list warning: {err}");
                continue;
            }
        };

        let entry_path = entry.path();
        let file_type = match entry.file_type() {
            Ok(file_type) => file_type,
            Err(err) => {
                eprintln!(
                    "FOD indexer source list warning for {}: {err}",
                    entry_path.display()
                );
                continue;
            }
        };
        if !file_type.is_dir() {
            continue;
        }
        if source::is_ignored_source_path(&root_path, &entry_path) {
            continue;
        }

        let canonical_path = match fs::canonicalize(&entry_path) {
            Ok(path) => path,
            Err(err) => {
                eprintln!(
                    "FOD indexer source list warning for {}: {err}",
                    entry_path.display()
                );
                continue;
            }
        };
        let added_sources = registered_by_root
            .get(&canonical_path)
            .cloned()
            .unwrap_or_default();
        entries.push(SourceBrowseEntry {
            path: canonical_path,
            added_sources,
        });
    }

    entries.sort_by(|a, b| a.path.cmp(&b.path));
    Ok((root_path, entries))
}
