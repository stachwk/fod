#!/usr/bin/env python3
from __future__ import annotations

import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
MAKEFILE = ROOT / "Makefile"


def target_line(text: str, target: str) -> str:
    prefix = f"{target}:"
    for line in text.splitlines():
        if line.startswith(prefix):
            return line
    raise AssertionError(f"missing Makefile target: {target}")


def target_block(text: str, target: str, next_target: str) -> str:
    start_marker = f"{target}:"
    end_marker = f"\n{next_target}:"
    start = text.find(start_marker)
    if start < 0:
        raise AssertionError(f"missing Makefile target: {target}")
    end = text.find(end_marker, start)
    if end < 0:
        raise AssertionError(
            f"missing target {next_target} after Makefile target {target}"
        )
    return text[start:end]


class MakefileDatabaseRestoreOrderTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.text = MAKEFILE.read_text(encoding="utf-8")

    def test_local_integration_suites_are_not_parallel(self) -> None:
        self.assertIn(
            ".NOTPARALLEL: test-integration test-all test-all-full",
            self.text,
        )

    def test_integration_uses_one_restored_mkfs_suite(self) -> None:
        line = target_line(self.text, "test-integration")
        self.assertIn("test-makefile-db-restore-order", line)
        self.assertIn("test-rust-mkfs-suite-local-restored", line)
        self.assertNotIn("test-rust-hotpath-runtime-size-limits", line)
        self.assertNotIn("test-runtime-validation", line)

    def test_wrapper_runs_suite_before_restore(self) -> None:
        block = target_block(
            self.text,
            "test-rust-mkfs-suite-local-restored",
            ".PHONY",
        )
        suite_command = (
            "$(MAKE) --no-print-directory test-rust-mkfs-suite "
            "|| suite_status=$$?"
        )
        restore_command = (
            "$(MAKE) --no-print-directory test-db-restore-local "
            "|| restore_status=$$?"
        )
        self.assertIn(suite_command, block)
        self.assertIn(restore_command, block)
        self.assertLess(block.index(suite_command), block.index(restore_command))
        self.assertIn('exit "$$suite_status"', block)
        self.assertIn('exit "$$restore_status"', block)

    def test_standalone_aliases_keep_their_original_scope(self) -> None:
        self.assertEqual(
            target_line(self.text, "test-runtime-validation"),
            "test-runtime-validation: test-rust-mkfs-suite",
        )
        self.assertEqual(
            target_line(self.text, "test-rust-hotpath-runtime-size-limits"),
            "test-rust-hotpath-runtime-size-limits: test-rust-mkfs-suite",
        )


if __name__ == "__main__":
    unittest.main()
