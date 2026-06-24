use std::fmt;

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
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
