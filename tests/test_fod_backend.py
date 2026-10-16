#!/usr/bin/env python
# -*- coding: utf-8 -*-
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1

from __future__ import annotations

import os
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))

import fod_backend


class FodBackendConfigBinaryTests(unittest.TestCase):
    def _binary(self, root: Path, relative: str) -> Path:
        path = root / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text("#!/bin/sh\nexit 0\n", encoding="utf-8")
        path.chmod(0o755)
        return path

    def test_explicit_fod_config_bin_has_priority(self) -> None:
        with tempfile.TemporaryDirectory(prefix="fod-backend-config-bin-") as tmp:
            root = Path(tmp)
            binary = self._binary(root, "explicit/fod-config")
            with (
                patch.object(fod_backend, "ROOT", root),
                patch.dict(
                    os.environ,
                    {"FOD_CONFIG_BIN": str(binary)},
                    clear=False,
                ),
            ):
                self.assertEqual(fod_backend._fod_config_cmd(), [str(binary)])

    def test_workspace_runtime_profile_binary_is_reused(self) -> None:
        with tempfile.TemporaryDirectory(prefix="fod-backend-runtime-profile-") as tmp:
            root = Path(tmp)
            binary = self._binary(root, "target/release-lto/fod-config")
            env = os.environ.copy()
            env.pop("FOD_CONFIG_BIN", None)
            env.pop("CARGO_TARGET_DIR", None)
            env["FOD_RUNTIME_PROFILE"] = "release-lto"
            with (
                patch.object(fod_backend, "ROOT", root),
                patch.dict(os.environ, env, clear=True),
            ):
                self.assertEqual(fod_backend._fod_config_cmd(), [str(binary)])

    def test_relative_cargo_target_dir_is_resolved_from_repo_root(self) -> None:
        with tempfile.TemporaryDirectory(prefix="fod-backend-target-dir-") as tmp:
            root = Path(tmp)
            binary = self._binary(root, "artifacts/release-lto/fod-config")
            env = os.environ.copy()
            env.pop("FOD_CONFIG_BIN", None)
            env["CARGO_TARGET_DIR"] = "artifacts"
            env["FOD_RUNTIME_PROFILE"] = "release-lto"
            with (
                patch.object(fod_backend, "ROOT", root),
                patch.dict(os.environ, env, clear=True),
            ):
                self.assertEqual(fod_backend._fod_config_cmd(), [str(binary)])

    def test_missing_explicit_binary_fails_without_cargo_fallback(self) -> None:
        with tempfile.TemporaryDirectory(prefix="fod-backend-missing-bin-") as tmp:
            root = Path(tmp)
            missing = root / "missing" / "fod-config"
            with (
                patch.object(fod_backend, "ROOT", root),
                patch.dict(
                    os.environ,
                    {"FOD_CONFIG_BIN": str(missing)},
                    clear=False,
                ),
            ):
                with self.assertRaisesRegex(
                    RuntimeError,
                    "FOD_CONFIG_BIN does not point to an existing fod-config binary",
                ):
                    fod_backend._fod_config_cmd()


if __name__ == "__main__":
    unittest.main()
