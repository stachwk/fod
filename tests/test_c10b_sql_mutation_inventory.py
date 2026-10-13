#!/usr/bin/env python3
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1

"""Offline tests for conservative C10-B2 SQL source inventory."""

from __future__ import annotations

import json
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))

from scripts.security.inventory_sql_mutations import collect, inventory_file, markdown


class SQLMutationInventoryTests(unittest.TestCase):
    def test_sql_and_dynamic_executor_are_reported_with_source_lines(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            path = root / "rust_hotpath/src/pg.rs"
            path.parent.mkdir(parents=True)
            path.write_text(
                """pub fn do_write(repo: &DbRepo) {
    // DELETE FROM files should not be emitted from comments
    let sql = "UPDATE files SET size = 42";
    validate_write_persistence_fence_on_conn();
    repo.exec(sql);
}
""",
                encoding="utf-8",
            )
            findings = inventory_file(path, root)
            sql = [
                item for item in findings
                if item["kind"] == "sql_mutation_candidate"
            ]
            sinks = [
                item for item in findings
                if item["kind"] == "sql_execution_sink"
            ]
            self.assertEqual(len(sql), 1)
            self.assertEqual(sql[0]["line"], 3)
            self.assertEqual(sql[0]["relation_hint"], "files")
            self.assertEqual(sql[0]["category"], "filesystem_state")
            self.assertEqual(sql[0]["symbol_hint"], "do_write")
            self.assertEqual(len(sinks), 1)
            self.assertEqual(sinks[0]["line"], 5)
            self.assertIn(
                "validate_write_persistence_fence_on_conn",
                sql[0]["guard_hints_not_proof"],
            )

    def test_migration_ddl_and_indexer_sql_are_reported(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            ddl = root / "migrations/0026_example.sql"
            ddl.parent.mkdir(parents=True)
            ddl.write_text(
                """-- DROP TABLE fake_comment;
CREATE TABLE IF NOT EXISTS file_write_leases (id BIGINT);
GRANT SELECT ON file_write_leases TO fod_runtime;
""",
                encoding="utf-8",
            )
            indexer = root / "rust_indexer/src/cleanup.rs"
            indexer.parent.mkdir(parents=True)
            indexer.write_text(
                """pub fn clean(repo: &DbRepo) {
    repo.exec("DELETE FROM files WHERE id_file = 1");
}
""",
                encoding="utf-8",
            )
            report = collect(root)
            self.assertEqual(report["files_scanned"], 2)
            self.assertEqual(
                [
                    (item["operation"], item["relation_hint"])
                    for item in report["findings"]
                    if item["kind"] == "sql_mutation_candidate"
                ],
                [
                    ("CREATE TABLE", "file_write_leases"),
                    ("GRANT", "SELECT"),
                    ("DELETE FROM", "files"),
                ],
            )
            self.assertTrue(any(
                item["kind"] == "sql_execution_sink"
                for item in report["findings"]
            ))
            # Report can be serialized and rendered without invoking PostgreSQL.
            json.dumps(report)
            self.assertIn("rust_indexer/src/cleanup.rs:2", markdown(report))

    def test_ignores_unscoped_paths_and_is_deterministic(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            outside = root / "docs/example.rs"
            outside.parent.mkdir(parents=True)
            outside.write_text('let s = "DELETE FROM files";', encoding="utf-8")
            self.assertEqual(collect(root)["findings_count"], 0)
            self.assertEqual(collect(root), collect(root))


if __name__ == "__main__":
    unittest.main()
