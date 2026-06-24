use fod_rust_runtime::ini_config::{load_config_parser, resolve_config_path_optional, IniConfig};
use fod_rust_runtime::parse_bool;
use std::collections::BTreeSet;
use std::path::Path;
use std::sync::OnceLock;

const INDEXER_SECTION: &str = "fod-indexer";

static INDEXER_SETTINGS: OnceLock<IndexerSettings> = OnceLock::new();

#[derive(Debug, Clone)]
pub struct IndexerSettings {
    pub skip_hidden: bool,
    pub skip_components: BTreeSet<String>,
    pub skip_prefixes: Vec<String>,
    pub allow_extensions: Option<BTreeSet<String>>,
}

impl Default for IndexerSettings {
    fn default() -> Self {
        Self {
            skip_hidden: true,
            skip_components: BTreeSet::new(),
            skip_prefixes: Vec::new(),
            allow_extensions: None,
        }
    }
}

impl IndexerSettings {
    fn from_ini(config: &IniConfig) -> Result<Self, String> {
        let mut settings = Self::default();
        let Some(section) = config.section(INDEXER_SECTION) else {
            return Ok(settings);
        };

        if let Some(value) = section.get("skip_hidden") {
            settings.skip_hidden = parse_bool(value)?;
        }

        if let Some(value) = section
            .get("skip_components")
            .or_else(|| section.get("ignore_components"))
        {
            settings.skip_components = parse_skip_components(value);
        }

        if let Some(value) = section
            .get("skip_prefixes")
            .or_else(|| section.get("ignore_prefixes"))
        {
            settings.skip_prefixes = parse_skip_prefixes(value);
        }

        if let Some(value) = section
            .get("skip_paths")
            .or_else(|| section.get("ignore_paths"))
        {
            for item in parse_path_rules(value) {
                if item.contains('/') || item.contains('\\') {
                    settings.skip_prefixes.push(normalize_prefix(&item));
                } else {
                    settings.skip_components.insert(normalize_component(&item));
                }
            }
        }

        if let Some(value) = section.get("allow_extensions") {
            settings.allow_extensions = parse_allow_extensions(value);
        }

        settings.skip_prefixes.sort();
        settings.skip_prefixes.dedup();
        Ok(settings)
    }

    pub fn allows_extension(&self, path: &Path) -> bool {
        match self.allow_extensions.as_ref() {
            None => true,
            Some(allowlist) => path
                .extension()
                .and_then(|value| value.to_str())
                .map(normalize_extension)
                .map_or(false, |extension| allowlist.contains(&extension)),
        }
    }
}

pub fn initialize_indexer_settings() -> Result<(), String> {
    let settings = load_indexer_settings()?;
    let _ = INDEXER_SETTINGS.set(settings);
    Ok(())
}

pub fn indexer_settings() -> &'static IndexerSettings {
    INDEXER_SETTINGS.get_or_init(IndexerSettings::default)
}

pub fn load_indexer_settings() -> Result<IndexerSettings, String> {
    let Some(config_path) = resolve_config_path_optional(None)? else {
        return Ok(IndexerSettings::default());
    };
    let (config, _) = load_config_parser(Some(&config_path))?;
    IndexerSettings::from_ini(&config)
}

fn parse_skip_components(value: &str) -> BTreeSet<String> {
    parse_list(value)
        .into_iter()
        .map(|item| normalize_component(&item))
        .collect()
}

fn parse_skip_prefixes(value: &str) -> Vec<String> {
    let mut prefixes = parse_list(value)
        .into_iter()
        .map(|item| normalize_prefix(&item))
        .collect::<Vec<_>>();
    prefixes.sort();
    prefixes.dedup();
    prefixes
}

fn parse_allow_extensions(value: &str) -> Option<BTreeSet<String>> {
    let extensions = parse_list(value)
        .into_iter()
        .map(|item| normalize_extension(&item))
        .filter(|item| !item.is_empty())
        .collect::<BTreeSet<_>>();
    if extensions.is_empty() {
        None
    } else {
        Some(extensions)
    }
}

fn parse_path_rules(value: &str) -> Vec<String> {
    parse_list(value)
}

fn parse_list(value: &str) -> Vec<String> {
    value
        .split(|ch: char| ch == ',' || ch == '\n' || ch == ';')
        .map(|item| item.trim())
        .filter(|item| !item.is_empty())
        .map(|item| item.to_string())
        .collect()
}

fn normalize_component(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

fn normalize_prefix(value: &str) -> String {
    value
        .trim()
        .replace('\\', "/")
        .trim_matches('/')
        .to_string()
}

fn normalize_extension(value: &str) -> String {
    value.trim().trim_start_matches('.').to_ascii_lowercase()
}
