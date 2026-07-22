#!/usr/bin/env python3

from __future__ import annotations

import os
import stat
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SCRIPT = ROOT / "tools" / "create_safe_pr.py"


class SafePrCreationTests(unittest.TestCase):
    def _run(
        self,
        body: str,
        *args: str,
        title: str = "Safe change",
    ) -> tuple[subprocess.CompletedProcess[str], Path, str]:
        temp_dir = tempfile.TemporaryDirectory()
        self.addCleanup(temp_dir.cleanup)
        root = Path(temp_dir.name)
        body_path = root / "pr-body.md"
        body_path.write_text(body, encoding="utf-8")

        capture_path = root / "gh-args.txt"
        fake_gh = root / "gh"
        fake_gh.write_text(
            "#!/bin/sh\n"
            "printf '%s\\n' \"$@\" > \"$SAFE_PR_CAPTURE\"\n",
            encoding="utf-8",
        )
        fake_gh.chmod(fake_gh.stat().st_mode | stat.S_IXUSR)

        env = os.environ.copy()
        env["PATH"] = f"{root}{os.pathsep}{env.get('PATH', '')}"
        env["SAFE_PR_CAPTURE"] = str(capture_path)

        command = [
            sys.executable,
            str(SCRIPT),
            "--title",
            title,
            "--body-file",
            str(body_path),
            *args,
        ]
        result = subprocess.run(
            command,
            cwd=ROOT,
            env=env,
            text=True,
            capture_output=True,
            check=False,
        )
        captured = capture_path.read_text(encoding="utf-8") if capture_path.exists() else ""
        return result, body_path, captured

    def test_dry_run_accepts_safe_text_without_invoking_gh(self) -> None:
        result, _, captured = self._run(
            "## Validation\n\n- 7 tests passed\n",
            "--dry-run",
        )
        self.assertEqual(result.returncode, 0)
        self.assertIn("safe PR text passed", result.stdout)
        self.assertEqual(captured, "")

    def test_sensitive_body_blocks_before_invoking_gh(self) -> None:
        sensitive = "/home/private-user/Documents/private-name.pdf"
        result, _, captured = self._run(f"Local result: {sensitive}\n")
        self.assertEqual(result.returncode, 1)
        self.assertEqual(captured, "")
        self.assertNotIn(sensitive, result.stdout)
        self.assertNotIn(sensitive, result.stderr)
        self.assertIn("unix-user-path", result.stderr)

    def test_sensitive_title_blocks_before_invoking_gh(self) -> None:
        sensitive = "/media/private-volume/private-directory"
        result, _, captured = self._run(
            "## Validation\n\n- passed\n",
            title=f"Inspect {sensitive}",
        )
        self.assertEqual(result.returncode, 1)
        self.assertEqual(captured, "")
        self.assertNotIn(sensitive, result.stdout)
        self.assertNotIn(sensitive, result.stderr)

    def test_invokes_gh_with_body_file_instead_of_body_contents(self) -> None:
        body = "## Validation\n\n- integration regression passed\n"
        result, body_path, captured = self._run(
            body,
            "--base",
            "main",
            "--head",
            "agent/example",
            "--repo",
            "example/project",
        )
        self.assertEqual(result.returncode, 0)
        captured_args = captured.splitlines()
        self.assertEqual(captured_args[:2], ["pr", "create"])
        self.assertIn("--body-file", captured_args)
        self.assertIn(str(body_path), captured_args)
        self.assertIn("--draft", captured_args)
        self.assertNotIn(body.strip(), captured)

    def test_ready_mode_omits_draft_flag(self) -> None:
        result, _, captured = self._run(
            "## Validation\n\n- passed\n",
            "--ready",
        )
        self.assertEqual(result.returncode, 0)
        self.assertNotIn("--draft", captured.splitlines())

    def test_unreadable_body_path_is_not_echoed(self) -> None:
        sensitive = "/home/private-user/private-body.md"
        result = subprocess.run(
            [
                sys.executable,
                str(SCRIPT),
                "--title",
                "Safe change",
                "--body-file",
                sensitive,
                "--dry-run",
            ],
            cwd=ROOT,
            text=True,
            capture_output=True,
            check=False,
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertNotIn(sensitive, result.stdout)
        self.assertNotIn(sensitive, result.stderr)
        self.assertIn("unable to read PR body file", result.stderr)


if __name__ == "__main__":
    unittest.main()
