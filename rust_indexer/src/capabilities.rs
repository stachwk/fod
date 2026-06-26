use std::fmt;

use serde::Serialize;

#[derive(Debug, Clone, Copy, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum SourcePolicy {
    PathBacked,
    Mirrored,
    ExportBacked,
}

impl fmt::Display for SourcePolicy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SourcePolicy::PathBacked => f.write_str("path-backed"),
            SourcePolicy::Mirrored => f.write_str("mirrored"),
            SourcePolicy::ExportBacked => f.write_str("export-backed"),
        }
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Serialize)]
pub struct SourceCapabilities {
    pub path_backed: bool,
    pub readonly: bool,
    pub mirror_required: bool,
    pub needs_export: bool,
    pub direct_crawler_possible: bool,
}

impl SourceCapabilities {
    pub const fn new(
        path_backed: bool,
        readonly: bool,
        mirror_required: bool,
        needs_export: bool,
        direct_crawler_possible: bool,
    ) -> Self {
        Self {
            path_backed,
            readonly,
            mirror_required,
            needs_export,
            direct_crawler_possible,
        }
    }

    pub const fn policy(self) -> SourcePolicy {
        if self.needs_export {
            SourcePolicy::ExportBacked
        } else if self.mirror_required {
            SourcePolicy::Mirrored
        } else {
            SourcePolicy::PathBacked
        }
    }
}

impl fmt::Display for SourceCapabilities {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "path_backed={} readonly={} mirror_required={} needs_export={} direct_crawler_possible={}",
            self.path_backed,
            self.readonly,
            self.mirror_required,
            self.needs_export,
            self.direct_crawler_possible
        )
    }
}
