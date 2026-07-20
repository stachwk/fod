use crate::capabilities::{SourceCapabilities, SourcePolicy};
use crate::cli::SourceKind;
use crate::model::{DuplicateSet, ImportPlanSummary, IndexSource, SourceBrowseEntry};
use fod_rust_runtime::FOD_VERSION_LABEL;
use serde::Serialize;

pub const INDEXER_API_SCHEMA_VERSION: u32 = 1;
pub const INDEXER_REQUIRED_DATABASE_SCHEMA_VERSION: u32 = 19;
pub const INDEXER_DEFAULT_PAGE_LIMIT: usize = 100;
pub const INDEXER_MAX_PAGE_LIMIT: usize = 1_000;

#[derive(Debug, Clone, Serialize)]
pub struct ProducerView {
    pub name: &'static str,
    pub version: &'static str,
}

impl ProducerView {
    fn current() -> Self {
        Self {
            name: "fod-indexer",
            version: FOD_VERSION_LABEL,
        }
    }
}

#[derive(Debug, Serialize)]
struct VersionedJsonOutput<'a, T>
where
    T: Serialize,
{
    pub schema_version: u32,
    pub producer: ProducerView,
    #[serde(flatten)]
    pub payload: &'a T,
}

impl<'a, T> VersionedJsonOutput<'a, T>
where
    T: Serialize,
{
    fn new(payload: &'a T) -> Self {
        Self {
            schema_version: INDEXER_API_SCHEMA_VERSION,
            producer: ProducerView::current(),
            payload,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct JsonOutputContractView {
    pub layout: &'static str,
    pub compatibility: &'static str,
    pub schema_version_field: &'static str,
    pub producer_field: &'static str,
}

#[derive(Debug, Clone, Serialize)]
pub struct CommandCapabilityView {
    pub command: &'static str,
    pub status: &'static str,
    pub read_only: bool,
    pub filters: &'static [&'static str],
    pub sort: &'static [&'static str],
    pub pagination: Option<&'static str>,
    pub default_limit: Option<usize>,
    pub max_limit: Option<usize>,
    pub consistency: &'static str,
    pub notes: &'static str,
}

#[derive(Debug, Clone, Serialize)]
pub struct IndexerCapabilitiesOutput {
    pub required_database_schema_version: u32,
    pub consistency_model: &'static str,
    pub output_formats: &'static [&'static str],
    pub json_contract: JsonOutputContractView,
    pub commands: Vec<CommandCapabilityView>,
}

impl IndexerCapabilitiesOutput {
    pub fn current() -> Self {
        Self {
            required_database_schema_version: INDEXER_REQUIRED_DATABASE_SCHEMA_VERSION,
            consistency_model: "live",
            output_formats: &["text", "json"],
            json_contract: JsonOutputContractView {
                layout: "flat-v1",
                compatibility: "existing payload fields remain at the top level; schema_version and producer are additive",
                schema_version_field: "schema_version",
                producer_field: "producer",
            },
            commands: vec![
                CommandCapabilityView {
                    command: "capabilities",
                    status: "available",
                    read_only: true,
                    filters: &[],
                    sort: &[],
                    pagination: None,
                    default_limit: None,
                    max_limit: None,
                    consistency: "static",
                    notes: "Does not require a PostgreSQL connection.",
                },
                CommandCapabilityView {
                    command: "source list",
                    status: "available",
                    read_only: true,
                    filters: &["kind", "path"],
                    sort: &["registered: kind,name,id", "browse: path"],
                    pagination: None,
                    default_limit: None,
                    max_limit: None,
                    consistency: "live",
                    notes: "Reads registered sources or browses a filesystem root without changing index state.",
                },
                CommandCapabilityView {
                    command: "plan show --id",
                    status: "available",
                    read_only: true,
                    filters: &["id"],
                    sort: &["entries: id_import_plan_entry"],
                    pagination: None,
                    default_limit: None,
                    max_limit: None,
                    consistency: "stored-snapshot",
                    notes: "Reads one already stored import plan and does not rebuild it.",
                },
                CommandCapabilityView {
                    command: "report duplicates --id",
                    status: "available",
                    read_only: true,
                    filters: &["id"],
                    sort: &["members: source_id,path-length,path"],
                    pagination: None,
                    default_limit: None,
                    max_limit: None,
                    consistency: "stored-derived-state",
                    notes: "Reads one existing duplicate set without rebuilding duplicate tables.",
                },
                CommandCapabilityView {
                    command: "report duplicates",
                    status: "available",
                    read_only: false,
                    filters: &["limit"],
                    sort: &["duplicate_set_id"],
                    pagination: Some("limit-only"),
                    default_limit: Some(INDEXER_DEFAULT_PAGE_LIMIT),
                    max_limit: None,
                    consistency: "refreshed-derived-state",
                    notes: "Rebuilds duplicate-set metadata before returning the report; consumers that require read-only behavior must not call it.",
                },
                CommandCapabilityView {
                    command: "plan list",
                    status: "planned-p0",
                    read_only: true,
                    filters: &["status", "cursor", "limit"],
                    sort: &["plan_id DESC"],
                    pagination: Some("keyset-cursor"),
                    default_limit: Some(INDEXER_DEFAULT_PAGE_LIMIT),
                    max_limit: Some(INDEXER_MAX_PAGE_LIMIT),
                    consistency: "live",
                    notes: "Will list stored plans without creating, refreshing, or modifying them.",
                },
                CommandCapabilityView {
                    command: "duplicate-set list",
                    status: "planned-p0",
                    read_only: true,
                    filters: &["cursor", "limit"],
                    sort: &["duplicate_set_id ASC"],
                    pagination: Some("keyset-cursor"),
                    default_limit: Some(INDEXER_DEFAULT_PAGE_LIMIT),
                    max_limit: Some(INDEXER_MAX_PAGE_LIMIT),
                    consistency: "live",
                    notes: "Will read existing duplicate metadata without invoking a rebuild.",
                },
                CommandCapabilityView {
                    command: "file list|search|show",
                    status: "planned-p0",
                    read_only: true,
                    filters: &[
                        "file_id",
                        "source",
                        "path",
                        "name",
                        "extension",
                        "file_kind",
                        "scan_status",
                        "hash_status",
                        "min_size",
                        "max_size",
                        "mtime_from",
                        "mtime_to",
                        "cursor",
                        "limit",
                    ],
                    sort: &["file_id ASC"],
                    pagination: Some("keyset-cursor"),
                    default_limit: Some(INDEXER_DEFAULT_PAGE_LIMIT),
                    max_limit: Some(INDEXER_MAX_PAGE_LIMIT),
                    consistency: "live",
                    notes: "Will expose stable index file ids and only fields already owned by fod-indexer; MIME and extracted-text metadata are not currently stored.",
                },
                CommandCapabilityView {
                    command: "file read --id",
                    status: "planned-p1",
                    read_only: true,
                    filters: &["id", "offset", "length"],
                    sort: &[],
                    pagination: Some("byte-range"),
                    default_limit: None,
                    max_limit: None,
                    consistency: "revalidated-source-read",
                    notes: "Will return revalidated source bytes; text extraction, MIME classification, embeddings, and OCR remain outside fod-indexer.",
                },
            ],
        }
    }

    pub fn human_readable(&self) -> String {
        let mut text = format!(
            "FOD indexer capabilities\nproducer: fod-indexer {}\napi schema: {}\nrequired database schema: {}\nconsistency model: {}\njson layout: {}\n",
            FOD_VERSION_LABEL,
            INDEXER_API_SCHEMA_VERSION,
            self.required_database_schema_version,
            self.consistency_model,
            self.json_contract.layout,
        );
        for command in &self.commands {
            text.push_str(&format!(
                "- command={} status={} read_only={} consistency={} filters={} sort={} pagination={} limits={}/{}\n  {}\n",
                command.command,
                command.status,
                command.read_only,
                command.consistency,
                display_values(command.filters),
                display_values(command.sort),
                command.pagination.unwrap_or("none"),
                command
                    .default_limit
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "none".to_string()),
                command
                    .max_limit
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "none".to_string()),
                command.notes,
            ));
        }
        text.trim_end().to_string()
    }
}

fn display_values(values: &[&str]) -> String {
    if values.is_empty() {
        "none".to_string()
    } else {
        values.join(",")
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct SourceRecordView {
    pub id_source: u64,
    pub name: String,
    pub kind: String,
    pub root_path: String,
    pub policy: Option<SourcePolicy>,
    pub capabilities: Option<SourceCapabilities>,
}

impl SourceRecordView {
    pub fn from_index_source(source: &IndexSource) -> Self {
        let (policy, capabilities) = source_kind_metadata(&source.kind)
            .map(|(policy, capabilities)| (Some(policy), Some(capabilities)))
            .unwrap_or((None, None));
        Self {
            id_source: source.id_source,
            name: source.name.clone(),
            kind: source.kind.clone(),
            root_path: source.root_path.display().to_string(),
            policy,
            capabilities,
        }
    }
}

impl From<&IndexSource> for SourceRecordView {
    fn from(source: &IndexSource) -> Self {
        Self::from_index_source(source)
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct SourceBrowseEntryView {
    pub path: String,
    pub added_sources: Vec<SourceRecordView>,
}

impl From<&SourceBrowseEntry> for SourceBrowseEntryView {
    fn from(entry: &SourceBrowseEntry) -> Self {
        Self {
            path: entry.path.display().to_string(),
            added_sources: entry
                .added_sources
                .iter()
                .map(SourceRecordView::from)
                .collect(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum SourceListMode {
    Registered,
    Browse,
    AdbShell,
}

#[derive(Debug, Clone, Serialize)]
pub struct SourceListOutput {
    pub mode: SourceListMode,
    pub kind_hint: Option<String>,
    pub root: Option<String>,
    pub device: Option<String>,
    pub adb_root: Option<String>,
    pub policy: Option<SourcePolicy>,
    pub capabilities: Option<SourceCapabilities>,
    pub registered_sources: Vec<SourceRecordView>,
    pub directories: Vec<SourceBrowseEntryView>,
}

impl SourceListOutput {
    pub fn registered(kind_hint: Option<String>, sources: Vec<IndexSource>) -> Self {
        Self {
            mode: SourceListMode::Registered,
            kind_hint,
            root: None,
            device: None,
            adb_root: None,
            policy: None,
            capabilities: None,
            registered_sources: sources.iter().map(SourceRecordView::from).collect(),
            directories: Vec::new(),
        }
    }

    pub fn browse(
        kind_hint: Option<String>,
        root: String,
        directories: Vec<SourceBrowseEntry>,
        policy: Option<SourcePolicy>,
        capabilities: Option<SourceCapabilities>,
    ) -> Self {
        Self {
            mode: SourceListMode::Browse,
            kind_hint,
            root: Some(root),
            device: None,
            adb_root: None,
            policy,
            capabilities,
            registered_sources: Vec::new(),
            directories: directories
                .iter()
                .map(SourceBrowseEntryView::from)
                .collect(),
        }
    }

    pub fn adb(
        device: String,
        adb_root: String,
        root: String,
        directories: Vec<SourceBrowseEntry>,
    ) -> Self {
        Self {
            mode: SourceListMode::AdbShell,
            kind_hint: Some(SourceKind::Adb.as_str().to_string()),
            root: Some(root),
            device: Some(device),
            adb_root: Some(adb_root),
            policy: Some(SourceKind::Adb.capabilities().policy()),
            capabilities: Some(SourceKind::Adb.capabilities()),
            registered_sources: Vec::new(),
            directories: directories
                .iter()
                .map(SourceBrowseEntryView::from)
                .collect(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct SourceMutationOutput {
    pub source: SourceRecordView,
}

#[derive(Debug, Clone, Serialize)]
pub struct DuplicateSetMemberView {
    pub file_id: u64,
    pub source_id: u64,
    pub source_name: String,
    pub source_kind: String,
    pub source_root_path: String,
    pub logical_path: String,
    pub source_path: String,
    pub size: u64,
    pub hash_algorithm: String,
    pub full_hash_hex: String,
    pub hash_status: String,
    pub is_canonical: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct DuplicateSetSnapshot {
    pub duplicate_set: DuplicateSet,
    pub members: Vec<DuplicateSetMemberView>,
}

impl DuplicateSetSnapshot {
    pub fn human_readable(&self) -> String {
        let canonical = self
            .members
            .iter()
            .find(|member| member.is_canonical)
            .or_else(|| self.members.first());
        let mut text = format!(
            "set {}: size={} files={} hash={} total_bytes={}",
            self.duplicate_set.id_duplicate_set,
            self.duplicate_set.file_size,
            self.duplicate_set.file_count,
            self.duplicate_set.full_hash_hex,
            self.duplicate_set.total_bytes
        );
        if let Some(canonical) = canonical {
            text.push_str(&format!(
                "\n  canonical: {}:{}",
                canonical.source_name, canonical.logical_path
            ));
        }
        for member in self.members.iter().filter(|member| !member.is_canonical) {
            text.push_str(&format!(
                "\n  reference: {}:{}",
                member.source_name, member.logical_path
            ));
        }
        text
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct DuplicateReportSnapshot {
    pub limit: Option<usize>,
    pub confirmed_duplicate_sets: u64,
    pub truncated: bool,
    pub duplicate_sets: Vec<DuplicateSetSnapshot>,
}

impl DuplicateReportSnapshot {
    pub fn human_readable(&self) -> String {
        let mut text = String::from("FOD indexer duplicate report\n");
        text.push_str(&format!(
            "confirmed duplicate sets: {}\n",
            self.confirmed_duplicate_sets
        ));
        for (idx, set) in self.duplicate_sets.iter().enumerate() {
            text.push_str(&set.human_readable());
            if idx + 1 < self.duplicate_sets.len() {
                text.push('\n');
            }
        }
        if self.truncated {
            if let Some(limit) = self.limit {
                text.push_str(&format!("\n... truncated after {limit} sets"));
            }
        }
        text.trim_end().to_string()
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ImportPlanEntryView {
    pub id_import_plan_entry: u64,
    pub id_import_plan: u64,
    pub id_file: u64,
    pub source_id: u64,
    pub source_name: String,
    pub source_kind: String,
    pub source_root_path: String,
    pub id_duplicate_set: Option<u64>,
    pub action: String,
    pub canonical_file_id: Option<u64>,
    pub logical_path: String,
    pub source_path: String,
    pub size: u64,
    pub mtime_ns: Option<i64>,
    pub source_changed: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ImportPlanSnapshot {
    pub summary: ImportPlanSummary,
    pub status: String,
    pub request_token: String,
    pub dry_run: bool,
    pub created_at: String,
    pub updated_at: String,
    pub entries: Vec<ImportPlanEntryView>,
}

impl ImportPlanSnapshot {
    pub fn human_readable(&self) -> String {
        let mut text = self.summary.human_readable();
        text.push_str(&format!(
            "\nstatus: {}\nrequest token: {}\ncreated at: {}\nupdated at: {}\nentries: {}",
            self.status,
            self.request_token,
            self.created_at,
            self.updated_at,
            self.entries.len()
        ));
        for entry in &self.entries {
            text.push_str(&format!(
                "\n- entry {}: action={} source={} kind={} path={} logical_path={} size={} source_changed={}",
                entry.id_import_plan_entry,
                entry.action,
                entry.source_name,
                entry.source_kind,
                entry.source_path,
                entry.logical_path,
                entry.size,
                entry.source_changed
            ));
        }
        text
    }
}

pub fn source_kind_metadata(kind: &str) -> Option<(SourcePolicy, SourceCapabilities)> {
    SourceKind::from_db_str(kind).map(|source_kind| {
        let capabilities = source_kind.capabilities();
        (capabilities.policy(), capabilities)
    })
}

pub fn print_json<T: Serialize>(value: &T) -> Result<(), String> {
    let text = serde_json::to_string_pretty(&VersionedJsonOutput::new(value))
        .map_err(|err| format!("unable to serialize JSON output: {err}"))?;
    println!("{text}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Serialize)]
    struct SampleOutput {
        value: u64,
    }

    #[test]
    fn versioned_json_keeps_existing_payload_fields_at_top_level() {
        let value = serde_json::to_value(VersionedJsonOutput::new(&SampleOutput { value: 7 }))
            .expect("versioned output should serialize");
        assert_eq!(
            value.get("schema_version").and_then(|value| value.as_u64()),
            Some(u64::from(INDEXER_API_SCHEMA_VERSION))
        );
        assert_eq!(
            value
                .get("producer")
                .and_then(|producer| producer.get("name"))
                .and_then(|value| value.as_str()),
            Some("fod-indexer")
        );
        assert_eq!(value.get("value").and_then(|value| value.as_u64()), Some(7));
        assert!(value.get("data").is_none());
        assert!(value.get("payload").is_none());
    }

    #[test]
    fn capabilities_distinguish_read_only_and_refreshing_commands() {
        let capabilities = IndexerCapabilitiesOutput::current();
        let plan_show = capabilities
            .commands
            .iter()
            .find(|command| command.command == "plan show --id")
            .expect("plan show capability should exist");
        assert!(plan_show.read_only);
        assert_eq!(plan_show.status, "available");

        let live_report = capabilities
            .commands
            .iter()
            .find(|command| command.command == "report duplicates")
            .expect("live duplicate report capability should exist");
        assert!(!live_report.read_only);

        let file_search = capabilities
            .commands
            .iter()
            .find(|command| command.command == "file list|search|show")
            .expect("planned file query capability should exist");
        assert_eq!(file_search.status, "planned-p0");
        assert_eq!(file_search.pagination, Some("keyset-cursor"));
    }
}
