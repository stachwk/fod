use crate::capabilities::{SourceCapabilities, SourcePolicy};
use crate::cli::SourceKind;
use crate::model::{DuplicateSet, ImportPlanSummary, IndexSource, SourceBrowseEntry};
use serde::Serialize;

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
    let text = serde_json::to_string_pretty(value)
        .map_err(|err| format!("unable to serialize JSON output: {err}"))?;
    println!("{text}");
    Ok(())
}
