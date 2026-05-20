#!/usr/bin/env python3
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1


from __future__ import annotations

from datetime import datetime, timezone


def db_timestamp_to_epoch(value: datetime) -> float:
    if value.tzinfo is None:
        value = value.replace(tzinfo=timezone.utc)
    return value.astimezone(timezone.utc).timestamp()


def epoch_to_utc_datetime(value: float) -> datetime:
    return datetime.fromtimestamp(value, tz=timezone.utc)
