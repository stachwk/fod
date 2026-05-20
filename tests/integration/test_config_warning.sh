#!/usr/bin/env bash
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

warn_output="$(
  make -s warn-config-secret FOD_CONFIG_SOURCE="${ROOT}/fod_config.ini"
)"
grep -Fq "Warning: ${ROOT}/fod_config.ini still contains password = cichosza." <<<"${warn_output}"
grep -Fq "use fod_config.example.ini for shared installs" <<<"${warn_output}"

example_output="$(
  make -s warn-config-secret FOD_CONFIG_SOURCE="${ROOT}/fod_config.example.ini"
)"
if grep -Fq "Warning:" <<<"${example_output}"; then
  printf '%s\n' "${example_output}"
  echo "unexpected warning for example config"
  exit 1
fi

echo "OK config warning"
