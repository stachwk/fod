#!/usr/bin/env python3

from __future__ import annotations

import contextlib
import importlib.util
import io
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
CHECKER_PATH = ROOT / "tools" / "check_github_text_privacy.py"
SPEC = importlib.util.spec_from_file_location("check_github_text_privacy", CHECKER_PATH)
if SPEC is None or SPEC.loader is None:
    raise RuntimeError(f"unable to load {CHECKER_PATH}")
CHECKER = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = CHECKER
SPEC.loader.exec_module(CHECKER)


class PrivacyCheckerTests(unittest.TestCase):
    def test_accepts_aggregate_validation_summary(self) -> None:
        text = """## Validation\n- 31 unit tests passed\n- integration regression passed\n- 3003 files scanned\n"""
        self.assertEqual(CHECKER.scan_text(text), [])

    def test_accepts_repository_and_synthetic_fixture_paths(self) -> None:
        text = """Changed rust_indexer/src/hash.rs.\nFixture: /tmp/fod-indexer-test-123/source-a/a.txt\n"""
        self.assertEqual(CHECKER.scan_text(text), [])

    def test_rejects_local_unix_paths_and_shell_prompt(self) -> None:
        text = """user@workstation:~/project$ command\npath=/home/user/Documents/private.pdf\nroot=/media/user/archive\n"""
        rules = {finding.rule for finding in CHECKER.scan_text(text)}
        self.assertIn("shell-prompt-identity", rules)
        self.assertIn("unix-user-path", rules)

    def test_rejects_raw_indexer_progress_and_current_file(self) -> None:
        text = """FOD indexer scan progress: phase=running scanned=50 current=secret.pdf status=ok\n"""
        rules = {finding.rule for finding in CHECKER.scan_text(text)}
        self.assertIn("raw-indexer-progress", rules)
        self.assertIn("current-file-field", rules)

    def test_rejects_catalogue_hashes_and_timestamps(self) -> None:
        text = """{
  \"full_hash_hex\": \"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\",
  \"created_at\": \"2026-01-01 12:00:00\"
}\n"""
        rules = {finding.rule for finding in CHECKER.scan_text(text)}
        self.assertIn("catalogue-hash-field", rules)
        self.assertIn("catalogue-timestamp-field", rules)

    def test_cli_does_not_echo_sensitive_value(self) -> None:
        sensitive = "/home/private-user/Documents/private-name.pdf"
        with tempfile.NamedTemporaryFile("w", encoding="utf-8", delete=False) as handle:
            handle.write(f"path={sensitive}\n")
            path = Path(handle.name)
        try:
            result = subprocess.run(
                [sys.executable, str(CHECKER_PATH), str(path)],
                cwd=ROOT,
                text=True,
                capture_output=True,
                check=False,
            )
        finally:
            path.unlink(missing_ok=True)

        self.assertEqual(result.returncode, 1)
        self.assertNotIn(sensitive, result.stdout)
        self.assertNotIn(sensitive, result.stderr)
        self.assertIn("unix-user-path", result.stderr)

    def test_cli_accepts_stdin(self) -> None:
        result = subprocess.run(
            [sys.executable, str(CHECKER_PATH), "--quiet"],
            cwd=ROOT,
            input="31 unit tests passed\n",
            text=True,
            capture_output=True,
            check=False,
        )
        self.assertEqual(result.returncode, 0)
        self.assertEqual(result.stdout, "")
        self.assertEqual(result.stderr, "")


if __name__ == "__main__":
    unittest.main()
