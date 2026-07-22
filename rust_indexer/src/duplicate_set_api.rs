use crate::output::{IndexerCapabilitiesOutput, INDEXER_MAX_PAGE_LIMIT};
use crate::read_api;
use fod_rust_hotpath::pg::DbRepo;
use serde::Serialize;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct DuplicateSetListItem {
    pub duplicate_set_id: u64,
    pub hash_algorithm: String,
    pub full_hash_hex: String,
    pub file_size: u64,
    pub file_count: u64,
    pub total_bytes: u64,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct DuplicateSetListOutput {
    pub consistency: &'static str,
    pub sort: &'static str,
    pub limit: usize,
    pub cursor: Option<u64>,
    pub items: Vec<DuplicateSetListItem>,
    pub next_cursor: Option<u64>,
    pub total: u64,
}

impl DuplicateSetListOutput {
    pub fn human_readable(&self) -> String {
        let mut text = format!(
            "FOD indexer duplicate sets\nconsistency: {}\nsort: {}\nlimit: {}\ncursor: {}\ntotal: {}\nitems: {}",
            self.consistency,
            self.sort,
            self.limit,
            self.cursor
                .map(|value| value.to_string())
                .unwrap_or_else(|| "none".to_string()),
            self.total,
            self.items.len(),
        );
        for item in &self.items {
            text.push_str(&format!(
                "\n- duplicate_set_id={} algorithm={} file_size={} file_count={} total_bytes={} hash={} created_at={} updated_at={}",
                item.duplicate_set_id,
                item.hash_algorithm,
                item.file_size,
                item.file_count,
                item.total_bytes,
                item.full_hash_hex,
                item.created_at,
                item.updated_at,
            ));
        }
        text.push_str(&format!(
            "\nnext_cursor: {}",
            self.next_cursor
                .map(|value| value.to_string())
                .unwrap_or_else(|| "none".to_string())
        ));
        text
    }
}

pub fn capabilities_output() -> IndexerCapabilitiesOutput {
    let mut capabilities = read_api::capabilities_output();
    if let Some(command) = capabilities
        .commands
        .iter_mut()
        .find(|command| command.command == "duplicate-set list")
    {
        command.status = "available";
        command.notes = "Lists existing duplicate-set metadata without scanning, hashing, rebuilding duplicate tables, or modifying index state.";
    }
    capabilities
}

pub fn load_duplicate_set_list(
    repo: &DbRepo,
    limit: usize,
    cursor: Option<u64>,
) -> Result<DuplicateSetListOutput, String> {
    validate_duplicate_set_list_request(limit, cursor)?;
    let fetch_limit = limit
        .checked_add(1)
        .ok_or_else(|| "duplicate-set list limit is too large".to_string())?;
    let where_clause = cursor
        .map(|value| format!("WHERE id_duplicate_set > {value}"))
        .unwrap_or_default();
    let rows = repo.query_rows_text(&format!(
        "
        SELECT
            id_duplicate_set::text,
            hash_algorithm,
            encode(full_hash, 'hex'),
            file_size::text,
            file_count::text,
            total_bytes::text,
            created_at::text,
            updated_at::text
        FROM index_duplicate_sets
        {where_clause}
        ORDER BY id_duplicate_set ASC
        LIMIT {fetch_limit}
        "
    ))?;
    let total_rows = repo.query_rows_text("SELECT COUNT(*) FROM index_duplicate_sets")?;
    let total = parse_count(&total_rows)?;

    let mut items = rows
        .iter()
        .map(|row| duplicate_set_list_item_from_row(row))
        .collect::<Result<Vec<_>, _>>()?;
    let has_more = items.len() > limit;
    if has_more {
        items.truncate(limit);
    }
    let next_cursor = if has_more {
        items.last().map(|item| item.duplicate_set_id)
    } else {
        None
    };

    Ok(DuplicateSetListOutput {
        consistency: "live",
        sort: "duplicate_set_id ASC",
        limit,
        cursor,
        items,
        next_cursor,
        total,
    })
}

fn validate_duplicate_set_list_request(limit: usize, cursor: Option<u64>) -> Result<(), String> {
    if !(1..=INDEXER_MAX_PAGE_LIMIT).contains(&limit) {
        return Err(format!(
            "duplicate-set list --limit must be between 1 and {INDEXER_MAX_PAGE_LIMIT}, got {limit}"
        ));
    }
    if matches!(cursor, Some(0)) {
        return Err("duplicate-set list --cursor must be a positive duplicate-set id".to_string());
    }
    Ok(())
}

fn duplicate_set_list_item_from_row(row: &[String]) -> Result<DuplicateSetListItem, String> {
    if row.len() < 8 {
        return Err("duplicate-set list row is too short".to_string());
    }
    Ok(DuplicateSetListItem {
        duplicate_set_id: parse_u64(&row[0], "duplicate-set id")?,
        hash_algorithm: row[1].clone(),
        full_hash_hex: row[2].clone(),
        file_size: parse_u64(&row[3], "duplicate-set file size")?,
        file_count: parse_u64(&row[4], "duplicate-set file count")?,
        total_bytes: parse_u64(&row[5], "duplicate-set total bytes")?,
        created_at: row[6].clone(),
        updated_at: row[7].clone(),
    })
}

fn parse_count(rows: &[Vec<String>]) -> Result<u64, String> {
    rows.first()
        .and_then(|row| row.first())
        .ok_or_else(|| "duplicate-set total row is missing".to_string())
        .and_then(|value| parse_u64(value, "duplicate-set total"))
}

fn parse_u64(value: &str, label: &str) -> Result<u64, String> {
    value
        .trim()
        .parse::<u64>()
        .map_err(|err| format!("invalid {label}: {err}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_duplicate_set_list_rows() {
        let row = vec![
            "7".to_string(),
            "sha256".to_string(),
            "abcdef".to_string(),
            "4096".to_string(),
            "3".to_string(),
            "12288".to_string(),
            "created".to_string(),
            "updated".to_string(),
        ];
        let item = duplicate_set_list_item_from_row(&row).expect("row should parse");
        assert_eq!(item.duplicate_set_id, 7);
        assert_eq!(item.hash_algorithm, "sha256");
        assert_eq!(item.full_hash_hex, "abcdef");
        assert_eq!(item.file_size, 4096);
        assert_eq!(item.file_count, 3);
        assert_eq!(item.total_bytes, 12288);
    }

    #[test]
    fn validates_duplicate_set_list_limits_and_cursor() {
        assert!(validate_duplicate_set_list_request(100, None).is_ok());
        assert!(validate_duplicate_set_list_request(0, None).is_err());
        assert!(validate_duplicate_set_list_request(INDEXER_MAX_PAGE_LIMIT + 1, None).is_err());
        assert!(validate_duplicate_set_list_request(10, Some(0)).is_err());
    }

    #[test]
    fn exposes_duplicate_set_list_as_available_capability() {
        let capabilities = capabilities_output();
        let command = capabilities
            .commands
            .iter()
            .find(|command| command.command == "duplicate-set list")
            .expect("duplicate-set capability should exist");
        assert_eq!(command.status, "available");
        assert!(command.read_only);
    }

    #[test]
    fn exposes_duplicate_set_show_as_available_capability() {
        let capabilities = capabilities_output();
        let command = capabilities
            .commands
            .iter()
            .find(|command| command.command == "duplicate-set show --id")
            .expect("duplicate-set show capability should exist");
        assert_eq!(command.status, "available");
        assert!(command.read_only);
        assert_eq!(command.consistency, "stored-derived-state");
    }
}
