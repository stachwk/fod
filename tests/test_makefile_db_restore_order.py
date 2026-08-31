#!/usr/bin/env python3
from __future__ import annotations

import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
MAKEFILE = ROOT / "make" / "fod-internal.mk"


def target_line(text: str, target: str) -> str:
    prefix = f"{target}:"
    for line in text.splitlines():
        if line.startswith(prefix):
            return line
    raise AssertionError(f"missing internal Make target: {target}")


def target_block(text: str, target: str, next_target: str) -> str:
    start_marker = f"{target}:"
    end_marker = f"\n{next_target}:"
    start = text.find(start_marker)
    if start < 0:
        raise AssertionError(f"missing internal Make target: {target}")
    end = text.find(end_marker, start)
    if end < 0:
        raise AssertionError(
            f"missing target {next_target} after internal Make target {target}"
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
        self.assertIn("test-rust-mkfs-suite-restored", line)
        self.assertNotIn("test-rust-mkfs-suite-local-restored", line)
        self.assertNotIn("test-rust-hotpath-runtime-size-limits", line)
        self.assertNotIn("test-runtime-validation", line)

    def test_wrapper_runs_suite_before_selected_restore(self) -> None:
        block = target_block(
            self.text,
            "test-rust-mkfs-suite-restored",
            ".PHONY",
        )
        suite_command = (
            "$(MAKE) --no-print-directory test-rust-mkfs-suite "
            "|| suite_status=$$?"
        )
        restore_command = (
            "$(MAKE) --no-print-directory test-db-restore-selected "
            "|| restore_status=$$?"
        )
        self.assertIn(
            "test-db-destructive-guard",
            target_line(self.text, "test-rust-mkfs-suite-restored"),
        )
        self.assertIn(suite_command, block)
        self.assertIn(restore_command, block)
        self.assertLess(block.index(suite_command), block.index(restore_command))
        self.assertIn('exit "$$suite_status"', block)
        self.assertIn('exit "$$restore_status"', block)

    def test_local_wrapper_forces_local_backend(self) -> None:
        block = target_block(
            self.text,
            "test-rust-mkfs-suite-local-restored",
            ".PHONY",
        )
        self.assertIn(
            "$(MAKE) --no-print-directory QNAP=0 test-rust-mkfs-suite-restored",
            block,
        )

    def test_destructive_schema_targets_restore_local_database(self) -> None:
        for target, next_target in [
            ("test-schema-upgrade", "test-schema-status"),
            ("test-schema-status", "test-df"),
        ]:
            block = target_block(self.text, target, next_target)
            self.assertIn("test_status=0", block)
            self.assertIn("restore_status=0", block)
            self.assertIn(
                "$(MAKE) --no-print-directory test-db-restore-selected "
                "|| restore_status=$$?",
                block,
            )
            self.assertIn(
                "$(MAKE) --no-print-directory test-db-destructive-guard",
                block,
            )
            self.assertIn('exit "$$test_status"', block)
            self.assertIn('exit "$$restore_status"', block)

    def test_qnap_reset_requires_explicit_destructive_opt_in(self) -> None:
        block = target_block(self.text, "reset", "test-db-destructive-guard")
        self.assertIn("QNAP_ALLOW_DESTRUCTIVE_RESET_ENABLED", block)
        self.assertIn("Refusing reset:", block)
        self.assertLess(
            block.index("Refusing reset:"),
            block.index("$(COMPOSE_RUN) -f $(COMPOSE_FILE) down -v"),
        )
        self.assertNotIn("\tsleep 2", block)

    def test_local_restore_uses_host_side_readiness_without_fixed_sleep(self) -> None:
        block = target_block(
            self.text,
            "test-db-restore-local",
            "test-db-restore-selected",
        )
        self.assertIn("$(MAKE) up QNAP=0", block)
        self.assertNotIn("\tsleep 2", block)

    def test_selected_restore_dispatches_by_backend(self) -> None:
        block = target_block(
            self.text,
            "test-db-restore-selected",
            "warn-config-secret",
        )
        self.assertIn(
            "test-db-destructive-guard",
            target_line(self.text, "test-db-restore-selected"),
        )
        self.assertIn(
            'reset QNAP="$(QNAP)" QNAP_ALLOW_DESTRUCTIVE_RESET="$(QNAP_ALLOW_DESTRUCTIVE_RESET)"',
            block,
        )
        self.assertIn("test-db-restore-local QNAP=0", block)

    def test_selected_backend_exports_complete_legacy_postgres_endpoint(self) -> None:
        self.assertIn("POSTGRES_HOST := $(FOD_PG_HOST)", self.text)
        for key in [
            "POSTGRES_HOST",
            "POSTGRES_PORT",
            "POSTGRES_DB",
            "POSTGRES_USER",
            "POSTGRES_PASSWORD",
        ]:
            self.assertIn(f"export {key}", self.text)
        self.assertLess(
            self.text.index("POSTGRES_HOST := $(FOD_PG_HOST)"),
            self.text.index("export POSTGRES_HOST"),
        )

    def test_up_waits_for_host_side_client_endpoint(self) -> None:
        block = target_block(self.text, "up", "docker-selinux-acl-up")
        self.assertIn("$(MAKE) wait QNAP=$(QNAP)", block)
        self.assertIn("$(MAKE) wait-client QNAP=$(QNAP)", block)
        client_wait = target_block(self.text, "wait-client", "init")
        self.assertIn('psql -v ON_ERROR_STOP=1 -h "$(FOD_PG_HOST)"', client_wait)
        self.assertIn("'SELECT 1'", client_wait)

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
