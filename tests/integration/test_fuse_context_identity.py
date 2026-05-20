#!/usr/bin/env python3
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1


from __future__ import annotations

import os
import sys
from unittest.mock import patch

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
if ROOT not in sys.path:
    sys.path.insert(0, ROOT)

import fod_identity


def main():
    with patch.dict(os.environ, {"FOD_USE_FUSE_CONTEXT": "1"}, clear=False), patch(
        "fuse.fuse_get_context", return_value=(1234, 2345, 4321)
    ), patch("fod_identity.os.kill", return_value=None):
        assert fod_identity.current_uid_gid(prefer_fuse_context=True) == (1234, 2345)
        assert fod_identity.current_group_ids(prefer_fuse_context=True) == {2345}

    print("OK fuse-context-identity")


if __name__ == "__main__":
    main()
