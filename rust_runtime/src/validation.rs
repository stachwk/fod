// Copyright (c) 2026 Wojciech Stach
// Licensed under BSL 1.1

//! Shared validation rules for configured runtime tuning and environment overrides.
//! Rule order controls which validation error is reported first.

use super::{lookup_value, parse_bool, parse_size_bytes};
use std::collections::HashMap;

pub fn validate_runtime_tuning(runtime: &HashMap<String, String>) -> Result<(), String> {
    validate_runtime_tuning_lookup(&|key| runtime.get(key).cloned())
}

fn validate_bool_value<F>(lookup: &F, key: &str) -> Result<(), String>
where
    F: Fn(&str) -> Option<String>,
{
    if let Some(value) = lookup_value(lookup, key) {
        parse_bool(&value).map_err(|err| format!("{}: {}", key, err))?;
    }
    Ok(())
}

fn validate_selinux_value<F>(lookup: &F, key: &str) -> Result<(), String>
where
    F: Fn(&str) -> Option<String>,
{
    if let Some(value) = lookup_value(lookup, key) {
        let normalized = value.trim().to_ascii_lowercase();
        if normalized != "auto" && parse_bool(&normalized).is_err() {
            return Err(format!("invalid {}: {}", key, value));
        }
    }
    Ok(())
}

fn validate_u64_value<F>(lookup: &F, key: &str, allow_zero: bool) -> Result<(), String>
where
    F: Fn(&str) -> Option<String>,
{
    if let Some(value) = lookup_value(lookup, key) {
        let parsed = value
            .parse::<u64>()
            .map_err(|_| format!("invalid {}: {}", key, value))?;
        if !allow_zero && parsed == 0 {
            return Err(format!("{} must be greater than zero", key));
        }
    }
    Ok(())
}

fn validate_duration_value<F>(lookup: &F, key: &str, allow_zero: bool) -> Result<(), String>
where
    F: Fn(&str) -> Option<String>,
{
    if let Some(value) = lookup_value(lookup, key) {
        let parsed = value
            .parse::<f64>()
            .map_err(|_| format!("invalid {}: {}", key, value))?;
        if !parsed.is_finite() || (!allow_zero && parsed <= 0.0) || (allow_zero && parsed < 0.0) {
            let comparator = if allow_zero {
                "greater than or equal to zero"
            } else {
                "greater than zero"
            };
            return Err(format!("{} must be {}", key, comparator));
        }
    }
    Ok(())
}

fn validate_choice_value<F>(lookup: &F, key: &str, allowed: &[&str]) -> Result<(), String>
where
    F: Fn(&str) -> Option<String>,
{
    if let Some(value) = lookup_value(lookup, key) {
        let normalized = value.trim().to_ascii_lowercase();
        if !allowed.contains(&normalized.as_str()) {
            return Err(format!("invalid {}: {}", key, value));
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RuntimeValidationTarget {
    Tuning,
    RuntimeEnvOverrides,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RuntimeValueKind {
    Bool,
    Selinux,
    Choice(&'static [&'static str]),
    U64 { allow_zero: bool },
    Duration { allow_zero: bool },
    SizeBytes { allow_zero: bool },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RuntimeValueSpec {
    key: &'static str,
    kind: RuntimeValueKind,
    validate_in_tuning: bool,
    validate_in_runtime_env_overrides: bool,
}

impl RuntimeValueSpec {
    const fn tuning_only(key: &'static str, kind: RuntimeValueKind) -> Self {
        Self {
            key,
            kind,
            validate_in_tuning: true,
            validate_in_runtime_env_overrides: false,
        }
    }

    const fn tuning_and_runtime_env(key: &'static str, kind: RuntimeValueKind) -> Self {
        Self {
            key,
            kind,
            validate_in_tuning: true,
            validate_in_runtime_env_overrides: true,
        }
    }

    const fn applies_to(self, target: RuntimeValidationTarget) -> bool {
        match target {
            RuntimeValidationTarget::Tuning => self.validate_in_tuning,
            RuntimeValidationTarget::RuntimeEnvOverrides => self.validate_in_runtime_env_overrides,
        }
    }
}

fn validate_runtime_value_spec<F>(lookup: &F, spec: &RuntimeValueSpec) -> Result<(), String>
where
    F: Fn(&str) -> Option<String>,
{
    match spec.kind {
        RuntimeValueKind::Bool => validate_bool_value(lookup, spec.key),
        RuntimeValueKind::Selinux => validate_selinux_value(lookup, spec.key),
        RuntimeValueKind::Choice(allowed) => validate_choice_value(lookup, spec.key, allowed),
        RuntimeValueKind::U64 { allow_zero } => validate_u64_value(lookup, spec.key, allow_zero),
        RuntimeValueKind::Duration { allow_zero } => {
            validate_duration_value(lookup, spec.key, allow_zero)
        }
        RuntimeValueKind::SizeBytes { allow_zero } => {
            if let Some(value) = lookup_value(lookup, spec.key) {
                let parsed =
                    parse_size_bytes(&value).map_err(|err| format!("{}: {}", spec.key, err))?;
                if !allow_zero && parsed == 0 {
                    return Err(format!("{} must be greater than zero", spec.key));
                }
            }
            Ok(())
        }
    }
}

fn validate_runtime_value_specs<F>(
    lookup: &F,
    target: RuntimeValidationTarget,
) -> Result<(), String>
where
    F: Fn(&str) -> Option<String>,
{
    for spec in RUNTIME_VALUE_SPECS {
        if spec.applies_to(target) {
            validate_runtime_value_spec(lookup, spec)?;
        }
    }
    Ok(())
}

const RUNTIME_VALUE_SPECS: &[RuntimeValueSpec] = &[
    RuntimeValueSpec::tuning_only(
        "role",
        RuntimeValueKind::Choice(&["auto", "primary", "replica"]),
    ),
    RuntimeValueSpec::tuning_and_runtime_env("force_read_only", RuntimeValueKind::Bool),
    RuntimeValueSpec::tuning_only("selinux", RuntimeValueKind::Selinux),
    RuntimeValueSpec::tuning_only("acl", RuntimeValueKind::Bool),
    RuntimeValueSpec::tuning_only(
        "atime_policy",
        RuntimeValueKind::Choice(&[
            "default",
            "noatime",
            "nodiratime",
            "relatime",
            "strictatime",
        ]),
    ),
    RuntimeValueSpec::tuning_only("default_permissions", RuntimeValueKind::Bool),
    RuntimeValueSpec::tuning_only("lazytime", RuntimeValueKind::Bool),
    RuntimeValueSpec::tuning_only("sync", RuntimeValueKind::Bool),
    RuntimeValueSpec::tuning_only("dirsync", RuntimeValueKind::Bool),
    RuntimeValueSpec::tuning_only(
        "log_level",
        RuntimeValueKind::Choice(&["off", "error", "warn", "info", "debug", "trace"]),
    ),
    RuntimeValueSpec::tuning_and_runtime_env("use_fuse_context", RuntimeValueKind::Bool),
    RuntimeValueSpec::tuning_and_runtime_env("fopen_direct_io", RuntimeValueKind::Bool),
    RuntimeValueSpec::tuning_and_runtime_env("fuse_writeback_cache", RuntimeValueKind::Bool),
    RuntimeValueSpec::tuning_and_runtime_env("use_rust_fuse", RuntimeValueKind::Bool),
    RuntimeValueSpec::tuning_and_runtime_env(
        "pool_max_connections",
        RuntimeValueKind::U64 { allow_zero: false },
    ),
    RuntimeValueSpec::tuning_and_runtime_env(
        "read_cache_blocks",
        RuntimeValueKind::U64 { allow_zero: true },
    ),
    RuntimeValueSpec::tuning_and_runtime_env(
        "read_cache_eviction_policy",
        RuntimeValueKind::Choice(&["fifo", "lru"]),
    ),
    RuntimeValueSpec::tuning_and_runtime_env(
        "read_ahead_blocks",
        RuntimeValueKind::U64 { allow_zero: true },
    ),
    RuntimeValueSpec::tuning_and_runtime_env(
        "sequential_read_ahead_blocks",
        RuntimeValueKind::U64 { allow_zero: true },
    ),
    RuntimeValueSpec::tuning_and_runtime_env(
        "direct_io_read_prefetch_blocks",
        RuntimeValueKind::U64 { allow_zero: true },
    ),
    RuntimeValueSpec::tuning_and_runtime_env(
        "small_file_read_threshold_blocks",
        RuntimeValueKind::U64 { allow_zero: true },
    ),
    RuntimeValueSpec::tuning_and_runtime_env(
        "workers_read",
        RuntimeValueKind::U64 { allow_zero: true },
    ),
    RuntimeValueSpec::tuning_and_runtime_env(
        "workers_read_min_blocks",
        RuntimeValueKind::U64 { allow_zero: true },
    ),
    RuntimeValueSpec::tuning_and_runtime_env(
        "workers_write",
        RuntimeValueKind::U64 { allow_zero: true },
    ),
    RuntimeValueSpec::tuning_and_runtime_env(
        "workers_write_min_blocks",
        RuntimeValueKind::U64 { allow_zero: true },
    ),
    RuntimeValueSpec::tuning_and_runtime_env(
        "persist_buffer_chunk_blocks",
        RuntimeValueKind::U64 { allow_zero: false },
    ),
    RuntimeValueSpec::tuning_and_runtime_env(
        "persist_block_transport",
        RuntimeValueKind::Choice(&["copy_binary_staging", "binary_bytea", "legacy_hex"]),
    ),
    RuntimeValueSpec::tuning_and_runtime_env(
        "data_object_swap_cleanup",
        RuntimeValueKind::Choice(&["immediate", "deferred"]),
    ),
    RuntimeValueSpec::tuning_and_runtime_env(
        "write_flush_threshold_bytes",
        RuntimeValueKind::SizeBytes { allow_zero: true },
    ),
    RuntimeValueSpec::tuning_and_runtime_env(
        "synchronous_commit",
        RuntimeValueKind::Choice(&[
            "on",
            "off",
            "local",
            "remote_write",
            "remote_apply",
            "true",
            "false",
        ]),
    ),
    RuntimeValueSpec::tuning_and_runtime_env(
        "lock_backend",
        RuntimeValueKind::Choice(&["memory", "postgres_lease"]),
    ),
    RuntimeValueSpec::tuning_and_runtime_env(
        "metadata_cache_ttl_seconds",
        RuntimeValueKind::Duration { allow_zero: true },
    ),
    RuntimeValueSpec::tuning_and_runtime_env(
        "statfs_cache_ttl_seconds",
        RuntimeValueKind::Duration { allow_zero: true },
    ),
    RuntimeValueSpec::tuning_and_runtime_env(
        "lock_lease_ttl_seconds",
        RuntimeValueKind::Duration { allow_zero: false },
    ),
    RuntimeValueSpec::tuning_and_runtime_env(
        "lock_heartbeat_interval_seconds",
        RuntimeValueKind::Duration { allow_zero: false },
    ),
    RuntimeValueSpec::tuning_and_runtime_env(
        "lock_poll_interval_seconds",
        RuntimeValueKind::Duration { allow_zero: false },
    ),
    RuntimeValueSpec::tuning_and_runtime_env("copy_dedupe_enabled", RuntimeValueKind::Bool),
    RuntimeValueSpec::tuning_and_runtime_env(
        "copy_dedupe_min_blocks",
        RuntimeValueKind::U64 { allow_zero: true },
    ),
    RuntimeValueSpec::tuning_and_runtime_env(
        "copy_dedupe_max_blocks",
        RuntimeValueKind::U64 { allow_zero: true },
    ),
    RuntimeValueSpec::tuning_and_runtime_env("copy_dedupe_crc_table", RuntimeValueKind::Bool),
    RuntimeValueSpec::tuning_and_runtime_env(
        "max_fs_size_bytes",
        RuntimeValueKind::SizeBytes { allow_zero: true },
    ),
];

pub(super) fn validate_runtime_tuning_lookup<F>(lookup: &F) -> Result<(), String>
where
    F: Fn(&str) -> Option<String>,
{
    validate_runtime_value_specs(lookup, RuntimeValidationTarget::Tuning)
}

pub(super) fn validate_runtime_env_overrides_lookup<F>(lookup: &F) -> Result<(), String>
where
    F: Fn(&str) -> Option<String>,
{
    validate_runtime_value_specs(lookup, RuntimeValidationTarget::RuntimeEnvOverrides)
}
