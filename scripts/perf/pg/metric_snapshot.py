#!/usr/bin/env python3
"""Shared helpers for key=value PostgreSQL profiling snapshots."""

from __future__ import annotations

from decimal import Decimal, InvalidOperation
from pathlib import Path


def parse_snapshot(path: Path) -> dict[str, str]:
    data: dict[str, str] = {}
    for raw_line in path.read_text(encoding="utf-8").splitlines():
        line = raw_line.strip()
        if not line or "=" not in line:
            continue
        key, value = line.split("=", 1)
        data[key.strip()] = value.strip()
    return data


def decimal_value(data: dict[str, str], key: str) -> Decimal:
    raw = data.get(key, "0")
    if raw == "":
        return Decimal(0)
    try:
        return Decimal(raw)
    except InvalidOperation as exc:
        raise ValueError(f"{key} is not numeric: {raw!r}") from exc


def format_decimal(value: Decimal) -> str:
    if value == value.to_integral_value():
        return str(int(value))
    return format(value.normalize(), "f")


def delta_value(before: dict[str, str], after: dict[str, str], key: str) -> Decimal:
    return decimal_value(after, key) - decimal_value(before, key)
