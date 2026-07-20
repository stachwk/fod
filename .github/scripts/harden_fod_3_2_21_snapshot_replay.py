from pathlib import Path


def replace_once(path: str, old: str, new: str) -> None:
    target = Path(path)
    text = target.read_text()
    count = text.count(old)
    if count != 1:
        raise SystemExit(f"{path}: expected one match, got {count}: {old[:120]!r}")
    target.write_text(text.replace(old, new, 1))


for path in ["migrations/0019_index_catalog_snapshots.sql", "migrations/base_schema.sql"]:
    replace_once(
        path,
        "CREATE TABLE IF NOT EXISTS index_catalog_snapshots (\n    id_catalog_snapshot SERIAL PRIMARY KEY,\n    status TEXT NOT NULL,",
        "CREATE TABLE IF NOT EXISTS index_catalog_snapshots (\n    id_catalog_snapshot SERIAL PRIMARY KEY,\n    request_token TEXT NOT NULL UNIQUE,\n    status TEXT NOT NULL,",
    )

replace_once(
    "rust_indexer/src/snapshot_api.rs",
    "use serde::Serialize;\n",
    "use serde::Serialize;\nuse std::sync::atomic::{AtomicU64, Ordering};\nuse std::time::{SystemTime, UNIX_EPOCH};\n\nstatic SNAPSHOT_REQUEST_TOKEN_COUNTER: AtomicU64 = AtomicU64::new(1);\n",
)

replace_once(
    "rust_indexer/src/snapshot_api.rs",
    "pub fn create_catalog_snapshot(\n",
    '''fn new_snapshot_request_token() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let sequence = SNAPSHOT_REQUEST_TOKEN_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!(
        "catalog-snapshot-{}-{nanos}-{sequence}",
        std::process::id()
    )
}

pub fn create_catalog_snapshot(
''',
)

replace_once(
    "rust_indexer/src/snapshot_api.rs",
    '''    ensure_snapshot_schema(repo, "snapshot create")?;
    let source = normalize_source_filter(source)?;
''',
    '''    ensure_snapshot_schema(repo, "snapshot create")?;
    let source = normalize_source_filter(source)?;
    let request_token = new_snapshot_request_token();
    let request_token_literal = sql_quote_literal(&request_token);
''',
)

replace_once(
    "rust_indexer/src/snapshot_api.rs",
    '''            INSERT INTO index_catalog_snapshots (
                status,
                source_filter,
                file_count,
                total_bytes,
                max_file_id
            )
            SELECT
                'complete',
                {source_literal},
                COUNT(*)::bigint,
                COALESCE(SUM(size), 0)::bigint,
                MAX(id_file)::bigint
            FROM source_rows
            RETURNING
                id_catalog_snapshot,
                status,
                source_filter,
                file_count,
                total_bytes,
                max_file_id,
                created_at
''',
    '''            INSERT INTO index_catalog_snapshots (
                request_token,
                status,
                source_filter,
                file_count,
                total_bytes,
                max_file_id
            )
            SELECT
                {request_token_literal},
                'complete',
                {source_literal},
                COUNT(*)::bigint,
                COALESCE(SUM(size), 0)::bigint,
                MAX(id_file)::bigint
            FROM source_rows
            ON CONFLICT (request_token) DO UPDATE
                SET request_token = EXCLUDED.request_token
            RETURNING
                id_catalog_snapshot,
                status,
                source_filter,
                file_count,
                total_bytes,
                max_file_id,
                created_at,
                (xmax = 0) AS created_new
''',
)

replace_once(
    "rust_indexer/src/snapshot_api.rs",
    '''            FROM source_rows
            CROSS JOIN created
            RETURNING id_file
''',
    '''            FROM source_rows
            CROSS JOIN created
            WHERE created.max_file_id IS NOT NULL
              AND source_rows.id_file <= created.max_file_id
            ON CONFLICT (id_catalog_snapshot, id_file) DO NOTHING
            RETURNING id_file
''',
)

replace_once(
    "rust_indexer/src/snapshot_api.rs",
    '''            COALESCE(created.max_file_id::text, ''),
            created.created_at::text,
            (SELECT COUNT(*) FROM copied)::text
''',
    '''            COALESCE(created.max_file_id::text, ''),
            created.created_at::text,
            created.created_new::text,
            (SELECT COUNT(*) FROM copied)::text
''',
)

replace_once(
    "rust_indexer/src/snapshot_api.rs",
    '''    if row.len() < 8 {
        return Err("catalog snapshot create row is too short".to_string());
    }
    let snapshot = snapshot_from_row(&row[..7])?;
    let copied_count = parse_u64(&row[7], "copied snapshot file count")?;
    if copied_count != snapshot.file_count {
        return Err(format!(
            "catalog_snapshot_incomplete: expected {} copied files, got {copied_count}",
            snapshot.file_count
        ));
    }
''',
    '''    if row.len() < 9 {
        return Err("catalog snapshot create row is too short".to_string());
    }
    let snapshot = snapshot_from_row(&row[..7])?;
    let created_new = parse_bool(&row[7]);
    let copied_count = parse_u64(&row[8], "copied snapshot file count")?;
    if created_new && copied_count != snapshot.file_count {
        return Err(format!(
            "catalog_snapshot_incomplete: expected {} copied files, got {copied_count}",
            snapshot.file_count
        ));
    }
    if !created_new && copied_count > snapshot.file_count {
        return Err(format!(
            "catalog_snapshot_replay_invalid: snapshot {} contains {} newly copied rows, expected at most {}",
            snapshot.snapshot_id, copied_count, snapshot.file_count
        ));
    }
''',
)

replace_once(
    "rust_indexer/src/snapshot_api.rs",
    '''        "SELECT
            to_regclass('fod.index_catalog_snapshots') IS NOT NULL,
            to_regclass('fod.index_catalog_snapshot_files') IS NOT NULL",
''',
    '''        "SELECT
            to_regclass('fod.index_catalog_snapshots') IS NOT NULL,
            to_regclass('fod.index_catalog_snapshot_files') IS NOT NULL,
            EXISTS (
                SELECT 1
                FROM information_schema.columns
                WHERE table_schema = 'fod'
                  AND table_name = 'index_catalog_snapshots'
                  AND column_name = 'request_token'
            )",
''',
)

replace_once(
    "rust_indexer/src/snapshot_api.rs",
    '''    let ready = rows
        .first()
        .is_some_and(|row| row.len() >= 2 && parse_bool(&row[0]) && parse_bool(&row[1]));
''',
    '''    let ready = rows.first().is_some_and(|row| {
        row.len() >= 3 && parse_bool(&row[0]) && parse_bool(&row[1]) && parse_bool(&row[2])
    });
''',
)

replace_once(
    "rust_indexer/src/snapshot_api.rs",
    '''    fn exposes_snapshot_capabilities() {
''',
    '''    fn request_tokens_are_unique_within_a_process() {
        let first = new_snapshot_request_token();
        let second = new_snapshot_request_token();
        assert_ne!(first, second);
        assert!(first.starts_with("catalog-snapshot-"));
    }

    #[test]
    fn exposes_snapshot_capabilities() {
''',
)

for path in ["docs/fod-indexer.md", "docs/fod-indexer-read-api.md"]:
    target = Path(path)
    text = target.read_text()
    old = "Snapshot creation and deletion write only snapshot tables. They do not scan, hash, materialize, read source bytes, or modify live index rows."
    new = old + " Snapshot creation uses an internal unique request token and conflict-safe replay bounded by the original maximum file id, so an ambiguous database retry cannot create a second snapshot or add later catalogue rows."
    if old not in text:
        raise SystemExit(f"{path}: snapshot documentation sentence not found")
    target.write_text(text.replace(old, new, 1))

uml = Path("uml/fod-catalog-snapshot-flow.puml")
text = uml.read_text()
old = "CLI -> DB: one atomic WITH statement\n"
new = "CLI -> DB: one atomic WITH statement\nunique request_token\n"
if old not in text:
    raise SystemExit("UML snapshot create message not found")
uml.write_text(text.replace(old, new, 1))
