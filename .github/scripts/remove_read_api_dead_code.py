from pathlib import Path

path = Path("rust_indexer/src/read_api.rs")
text = path.read_text(encoding="utf-8")

blocks = [
    '''impl FileCatalogItem {
    fn human_readable(&self) -> String {
        format!(
            "file_id={} source={} kind={} path={} size={} mtime_ns={} file_kind={} scan_status={} hash_status={} hash={} source_changed={} scan_run_id={}",
            self.file_id,
            self.source_name,
            self.source_kind,
            self.path,
            self.size,
            self.mtime_ns
                .map(|value| value.to_string())
                .unwrap_or_else(|| "none".to_string()),
            self.file_kind,
            self.scan_status,
            self.hash_status.as_deref().unwrap_or("none"),
            self.full_hash_hex.as_deref().unwrap_or("none"),
            self.source_changed,
            self.scan_run_id
                .map(|value| value.to_string())
                .unwrap_or_else(|| "none".to_string()),
        )
    }
}

''',
    '''impl FileCatalogOutput {
    pub fn human_readable(&self) -> String {
        let mut text = format!(
            "FOD indexer file catalogue\\nconsistency: {}\\nsort: {}\\nlimit: {}\\ncursor: {}\\ntotal: {}\\nitems: {}",
            self.consistency,
            self.sort,
            self.limit,
            self.cursor
                .map(|value| value.to_string())
                .unwrap_or_else(|| "none".to_string()),
            self.total,
            self.items.len(),
        );
        if !self.filters.is_empty() {
            text.push_str(&format!("\\nfilters: {:?}", self.filters));
        }
        for item in &self.items {
            text.push_str("\\n- ");
            text.push_str(&item.human_readable());
        }
        text.push_str(&format!(
            "\\nnext_cursor: {}",
            self.next_cursor
                .map(|value| value.to_string())
                .unwrap_or_else(|| "none".to_string())
        ));
        text
    }
}

''',
    '''impl FileShowOutput {
    pub fn human_readable(&self) -> String {
        format!(
            "FOD indexer file\\nconsistency: {}\\n{}\\nsource_root={}\\nsource_path={}\\nname={}\\nextension={}\\nhash_algorithm={}\\ncreated_at={}\\nupdated_at={}",
            self.consistency,
            self.item.human_readable(),
            self.item.source_root,
            self.item.source_path,
            self.item.name,
            self.item.extension.as_deref().unwrap_or("none"),
            self.item.hash_algorithm.as_deref().unwrap_or("none"),
            self.item.created_at,
            self.item.updated_at,
        )
    }
}

''',
]

for block in blocks:
    count = text.count(block)
    if count != 1:
        raise SystemExit(f"expected exactly one dead-code block, found {count}")
    text = text.replace(block, "", 1)

path.write_text(text, encoding="utf-8")
