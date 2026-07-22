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
        git_branch: str = "agent/test-change",
        git_ahead: str = "1",
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

        fake_git = root / "git"
        fake_git.write_text(
            "#!/bin/sh\n"
            "if [ \"$1\" = branch ] && [ \"$2\" = --show-current ]; then\n"
            "  printf '%s\\n' \"$SAFE_PR_GIT_BRANCH\"\n"
            "  exit 0\n"
            "fi\n"
            "if [ \"$1\" = rev-list ] && [ \"$2\" = --count ]; then\n"
            "  printf '%s\\n' \"$SAFE_PR_GIT_AHEAD\"\n"
            "  exit 0\n"
            "fi\n"
            "exit 2\n",
            encoding="utf-8",
        )
        fake_git.chmod(fake_git.stat().st_mode | stat.S_IXUSR)

        env = os.environ.copy()
        env["PATH"] = f"{root}{os.pathsep}{env.get('PATH', '')}"
        env["SAFE_PR_CAPTURE"] = str(capture_path)
        env["SAFE_PR_GIT_BRANCH"] = git_branch
        env["SAFE_PR_GIT_AHEAD"] = git_ahead

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
            git_branch="main",
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

    def test_invokes_gh_with_body_file_and_validated_branches(self) -> None:
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
        self.assertIn("--base", captured_args)
        self.assertIn("main", captured_args)
        self.assertIn("--head", captured_args)
        self.assertIn("agent/example", captured_args)
        self.assertIn("--draft", captured_args)
        self.assertNotIn(body.strip(), captured)

    def test_current_branch_is_used_when_head_is_omitted(self) -> None:
        result, _, captured = self._run(
            "## Validation\n\n- passed\n",
            git_branch="agent/inferred-branch",
        )
        self.assertEqual(result.returncode, 0)
        captured_args = captured.splitlines()
        head_index = captured_args.index("--head")
        self.assertEqual(captured_args[head_index + 1], "agent/inferred-branch")

    def test_ready_mode_omits_draft_flag(self) -> None:
        result, _, captured = self._run(
            "## Validation\n\n- passed\n",
            "--ready",
        )
        self.assertEqual(result.returncode, 0)
        self.assertNotIn("--draft", captured.splitlines())

    def test_base_branch_is_blocked_before_invoking_gh(self) -> None:
        result, _, captured = self._run(
            "## Validation\n\n- passed\n",
            git_branch="main",
        )
        self.assertEqual(result.returncode, 1)
        self.assertEqual(captured, "")
        self.assertIn("head branch matches the base branch", result.stderr)

    def test_explicit_head_matching_base_is_blocked(self) -> None:
        result, _, captured = self._run(
            "## Validation\n\n- passed\n",
            "--base",
            "main",
            "--head",
            "main",
        )
        self.assertEqual(result.returncode, 1)
        self.assertEqual(captured, "")
        self.assertIn("head branch matches the base branch", result.stderr)

    def test_branch_without_commits_ahead_is_blocked(self) -> None:
        result, _, captured = self._run(
            "## Validation\n\n- passed\n",
            git_ahead="0",
        )
        self.assertEqual(result.returncode, 1)
        self.assertEqual(captured, "")
        self.assertIn("no commits ahead of base", result.stderr)

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

    def test_prepare_body_copies_repository_template(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            template_path = root / ".github" / "PULL_REQUEST_TEMPLATE.md"
            template_path.parent.mkdir(parents=True)
            template_path.write_text("## Summary\n\n- \n", encoding="utf-8")

            result = subprocess.run(
                [sys.executable, str(SCRIPT), "--prepare-body"],
                cwd=root,
                text=True,
                capture_output=True,
                check=False,
            )

            body_path = root / ".git" / "fod-private" / "pr-body.md"
            self.assertEqual(result.returncode, 0)
            self.assertEqual(body_path.read_text(encoding="utf-8"), "## Summary\n\n- \n")
            self.assertIn("safe PR body prepared", result.stdout)

    def test_prepare_body_does_not_overwrite_existing_file(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            template_path = root / ".github" / "PULL_REQUEST_TEMPLATE.md"
            template_path.parent.mkdir(parents=True)
            template_path.write_text("template\n", encoding="utf-8")
            body_path = root / ".git" / "fod-private" / "pr-body.md"
            body_path.parent.mkdir(parents=True)
            body_path.write_text("custom\n", encoding="utf-8")

            result = subprocess.run(
                [sys.executable, str(SCRIPT), "--prepare-body"],
                cwd=root,
                text=True,
                capture_output=True,
                check=False,
            )

            self.assertEqual(result.returncode, 0)
            self.assertEqual(body_path.read_text(encoding="utf-8"), "custom\n")
            self.assertIn("not overwritten", result.stdout)


if __name__ == "__main__":
    unittest.main()
