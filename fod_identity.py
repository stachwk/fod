#!/usr/bin/env python3
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1


from __future__ import annotations

import os


def _use_fuse_context(prefer_fuse_context: bool) -> bool:
    if prefer_fuse_context:
        return True
    return os.environ.get("FOD_USE_FUSE_CONTEXT", "").lower() in {"1", "true", "yes", "on"}


def _current_fuse_context() -> tuple[int, int] | None:
    try:
        import fuse  # type: ignore
    except Exception:
        return None

    try:
        uid, gid, pid = fuse.fuse_get_context()
    except Exception:
        return None

    if pid:
        try:
            os.kill(pid, 0)
        except OSError:
            pass

    return int(uid), int(gid)


def current_uid_gid(*, prefer_fuse_context: bool = False) -> tuple[int, int]:
    if _use_fuse_context(prefer_fuse_context):
        context = _current_fuse_context()
        if context is not None:
            return context
    return os.getuid(), os.getgid()


def current_group_ids(*, prefer_fuse_context: bool = False) -> set[int]:
    if _use_fuse_context(prefer_fuse_context):
        _, gid = current_uid_gid(prefer_fuse_context=True)
        return {gid}
    groups = set(os.getgroups())
    groups.add(os.getgid())
    return groups
