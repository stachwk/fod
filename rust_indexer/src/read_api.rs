use crate::db::sql_quote_literal;
use crate::output::{IndexerCapabilitiesOutput, INDEXER_MAX_PAGE_LIMIT};
use fod_rust_hotpath::pg::DbRepo;
use serde::Serialize;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ImportPlanListItem {
    pub plan_id: u64,
    pub status: String,
    pub source_filter: Option<String>,
    pub dry_run: bool,
    pub created_at: String,
    pub updated_at: String,
    pub entry_count: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct ImportPlanListOutput {
    pub consistency: &'static str,
    pub sort: &'static str,
    pub limit: usize,
    pub cursor: Option<u64>,
    pub status_filter: Option<String>,
    pub items: Vec<ImportPlanListItem>,
    pub next_cursor: Option<u64>,
    pub total: u64,
}

impl ImportPlanListOutput {
    pub fn human_readable(&self) -> String {
        let mut text = format!(
            "FOD indexer import plans\nconsistency: {}\nsort: {}\nlimit: {}\ncursor: {}\nstatus filter: {}\ntotal: {}\nitems: {}",
            self.consistency,
            self.sort,
            self.limit,
            self.cursor
                .map(|value| value.to_string())
                .unwrap_or_else(|| "none".to_string()),
            self.status_filter.as_deref().unwrap_or("all"),
            self.total,
            self.items.len(),
        );
        for item in &self.items {
            text.push_str(&format!(
                "\n- plan_id={} status={} source_filter={} dry_run={} created_at={} updated_at={} entry_count={}",
                item.plan_id,
                item.status,
                item.source_filter.as_deref().unwrap_or("all"),
                item.dry_run,
                item.created_at,
                item.updated_at,
                item.entry_count,
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
    let mut capabilities = IndexerCapabilitiesOutput::current();
    if let Some(command) = capabilities
        .commands
        .iter_mut()
        .find(|command| command.command == "plan list")
    {
        command.status = "available";
        command.notes =
            "Lists stored import plans without creating, refreshing, or modifying them.";
    }
    capabilities
}

pub fn load_import_plan_list(
    repo: &DbRepo,
    limit: usize,
    cursor: Option<u64>,
    status: Option<&str>,
) -> Result<ImportPlanListOutput, String> {
    validate_plan_list_request(limit, cursor, status)?;

    let normalized_status = status.map(str::trim).filter(|value| !value.is_empty());
    let mut page_conditions = Vec::new();
    if let Some(cursor) = cursor {
        page_conditions.push(format!("p.id_import_plan < {cursor}"));
    }
    if let Some(status) = normalized_status {
        page_conditions.push(format!("p.status = {}", sql_quote_literal(status)));
    }
    let page_where = if page_conditions.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", page_conditions.join(" AND "))
    };

    let fetch_limit = limit
        .checked_add(1)
        .ok_or_else(|| "plan list limit is too large".to_string())?;
    let rows = repo.query_rows_text(&format!(
        "
        SELECT
            p.id_import_plan,
            p.status,
            COALESCE(p.source_filter, ''),
            p.dry_run::text,
            p.created_at::text,
            p.updated_at::text,
            COUNT(e.id_import_plan_entry)
        FROM index_import_plans p
        LEFT JOIN index_import_plan_entries e
            ON e.id_import_plan = p.id_import_plan
        {page_where}
        GROUP BY
            p.id_import_plan,
            p.status,
            p.source_filter,
            p.dry_run,
            p.created_at,
            p.updated_at
        ORDER BY p.id_import_plan DESC
        LIMIT {fetch_limit}
        "
    ))?;

    let total_where = normalized_status
        .map(|status| format!("WHERE status = {}", sql_quote_literal(status)))
        .unwrap_or_default();
    let total_rows = repo.query_rows_text(&format!(
        "
        SELECT COUNT(*)
        FROM index_import_plans
        {total_where}
        "
    ))?;
    let total = total_rows
        .first()
        .and_then(|row| row.first())
        .ok_or_else(|| "import plan total row is missing".to_string())?
        .trim()
        .parse::<u64>()
        .map_err(|err| format!("invalid import plan total: {err}"))?;

    let mut items = rows
        .iter()
        .map(|row| import_plan_list_item_from_row(row))
        .collect::<Result<Vec<_>, _>>()?;
    let has_more = items.len() > limit;
    if has_more {
        items.truncate(limit);
    }
    let next_cursor = if has_more {
        items.last().map(|item| item.plan_id)
    } else {
        None
    };

    Ok(ImportPlanListOutput {
        consistency: "live",
        sort: "plan_id DESC",
        limit,
        cursor,
        status_filter: normalized_status.map(str::to_string),
        items,
        next_cursor,
        total,
    })
}

fn validate_plan_list_request(
    limit: usize,
    cursor: Option<u64>,
    status: Option<&str>,
) -> Result<(), String> {
    if !(1..=INDEXER_MAX_PAGE_LIMIT).contains(&limit) {
        return Err(format!(
            "plan list --limit must be between 1 and {INDEXER_MAX_PAGE_LIMIT}, got {limit}"
        ));
    }
    if matches!(cursor, Some(0)) {
        return Err("plan list --cursor must be a positive plan id".to_string());
    }
    if status.is_some_and(|value| value.trim().is_empty()) {
        return Err("plan list --status must not be empty".to_string());
    }
    Ok(())
}

fn import_plan_list_item_from_row(row: &[String]) -> Result<ImportPlanListItem, String> {
    if row.len() < 7 {
        return Err("import plan list row is too short".to_string());
    }
    Ok(ImportPlanListItem {
        plan_id: parse_u64(&row[0], "import plan id")?,
        status: row[1].clone(),
        source_filter: if row[2].trim().is_empty() {
            None
        } else {
            Some(row[2].clone())
        },
        dry_run: parse_bool(&row[3]),
        created_at: row[4].clone(),
        updated_at: row[5].clone(),
        entry_count: parse_u64(&row[6], "import plan entry count")?,
    })
}

fn parse_u64(value: &str, label: &str) -> Result<u64, String> {
    value
        .trim()
        .parse::<u64>()
        .map_err(|err| format!("invalid {label}: {err}"))
}

fn parse_bool(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "t" | "true" | "1" | "on"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_import_plan_list_rows() {
        let row = vec![
            "42".to_string(),
            "dry_run_completed".to_string(),
            "lt7300_Documents".to_string(),
            "true".to_string(),
            "2026-07-19 20:00:00+00".to_string(),
            "2026-07-19 20:01:00+00".to_string(),
            "135".to_string(),
        ];
        let item = import_plan_list_item_from_row(&row).expect("row should parse");
        assert_eq!(item.plan_id, 42);
        assert_eq!(item.status, "dry_run_completed");
        assert_eq!(item.source_filter.as_deref(), Some("lt7300_Documents"));
        assert!(item.dry_run);
        assert_eq!(item.entry_count, 135);
    }

    #[test]
    fn parses_empty_source_filter_as_none() {
        let row = vec![
            "7".to_string(),
            "planned".to_string(),
            String::new(),
            "false".to_string(),
            "created".to_string(),
            "updated".to_string(),
            "0".to_string(),
        ];
        let item = import_plan_list_item_from_row(&row).expect("row should parse");
        assert_eq!(item.source_filter, None);
        assert!(!item.dry_run);
    }

    #[test]
    fn validates_plan_list_limits_and_cursor() {
        assert!(validate_plan_list_request(
            crate::output::INDEXER_DEFAULT_PAGE_LIMIT,
            None,
            None
        )
        .is_ok());
        assert!(validate_plan_list_request(0, None, None).is_err());
        assert!(validate_plan_list_request(INDEXER_MAX_PAGE_LIMIT + 1, None, None).is_err());
        assert!(validate_plan_list_request(10, Some(0), None).is_err());
        assert!(validate_plan_list_request(10, None, Some(" ")).is_err());
    }

    #[test]
    fn exposes_plan_list_as_available_capability() {
        let capabilities = capabilities_output();
        let plan_list = capabilities
            .commands
            .iter()
            .find(|command| command.command == "plan list")
            .expect("plan list capability should exist");
        assert_eq!(plan_list.status, "available");
        assert!(plan_list.read_only);
    }
}
