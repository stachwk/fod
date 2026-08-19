// Copyright (c) 2026 Wojciech Stach
// Licensed under BSL 1.1

use fod_rust_runtime::{env_var_with_legacy_alias, parse_size_bytes};
use fuser::{InitFlags, KernelConfig, Version};
use log::{info, warn};
use std::io;

pub(crate) const FUSER_VERSION: &str = "0.18.0";
pub(crate) const USERSPACE_PROTOCOL_MAX: Version = Version(7, 40);
pub(crate) const DEFAULT_FUSE_MAX_WRITE_BYTES: u32 = 512 * 1024;
pub(crate) const DEFAULT_FUSE_MAX_READAHEAD_BYTES: u32 = 512 * 1024;

const FUSE_MAX_WRITE_ENV: &str = "FOD_FUSE_MAX_WRITE_BYTES";
const FUSE_MAX_READAHEAD_ENV: &str = "FOD_FUSE_MAX_READAHEAD_BYTES";
const FOD_REQUESTED_CAPABILITIES: InitFlags =
    InitFlags::FUSE_POSIX_LOCKS.union(InitFlags::FUSE_FLOCK_LOCKS);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct FuseCompatibilitySnapshot {
    pub(crate) kernel_protocol: Version,
    pub(crate) negotiated_protocol: Version,
    pub(crate) available_capabilities: InitFlags,
    pub(crate) requested_capabilities: InitFlags,
    pub(crate) enabled_capabilities: InitFlags,
    pub(crate) unsupported_capabilities: InitFlags,
    pub(crate) requested_max_write: u32,
    pub(crate) effective_max_write: u32,
    pub(crate) requested_max_readahead: u32,
    pub(crate) effective_max_readahead: u32,
}

impl FuseCompatibilitySnapshot {
    fn from_parts(
        kernel_protocol: Version,
        available_capabilities: InitFlags,
        requested_max_write: u32,
        effective_max_write: u32,
        requested_max_readahead: u32,
        effective_max_readahead: u32,
    ) -> Self {
        let requested_capabilities = FOD_REQUESTED_CAPABILITIES;
        let enabled_capabilities = requested_capabilities & available_capabilities;
        let unsupported_capabilities = requested_capabilities & !available_capabilities;
        Self {
            kernel_protocol,
            negotiated_protocol: kernel_protocol.min(USERSPACE_PROTOCOL_MAX),
            available_capabilities,
            requested_capabilities,
            enabled_capabilities,
            unsupported_capabilities,
            requested_max_write,
            effective_max_write,
            requested_max_readahead,
            effective_max_readahead,
        }
    }

    pub(crate) fn configure(config: &mut KernelConfig) -> io::Result<Self> {
        let kernel_protocol = config.kernel_abi();
        let available_capabilities = config.capabilities();
        let requested_max_write =
            requested_limit_from_env(FUSE_MAX_WRITE_ENV, DEFAULT_FUSE_MAX_WRITE_BYTES)?;
        let requested_max_readahead =
            requested_limit_from_env(FUSE_MAX_READAHEAD_ENV, DEFAULT_FUSE_MAX_READAHEAD_BYTES)?;

        let effective_max_write =
            apply_limit_with_fallback("max_write", requested_max_write, false, |value| {
                config.set_max_write(value)
            })?;
        let effective_max_readahead =
            apply_limit_with_fallback("max_readahead", requested_max_readahead, true, |value| {
                config.set_max_readahead(value)
            })?;

        let snapshot = Self::from_parts(
            kernel_protocol,
            available_capabilities,
            requested_max_write,
            effective_max_write,
            requested_max_readahead,
            effective_max_readahead,
        );
        if !snapshot.enabled_capabilities.is_empty() {
            config
                .add_capabilities(snapshot.enabled_capabilities)
                .map_err(|unexpected| {
                    io::Error::other(format!(
                        "fuser rejected capabilities reported as available: {}",
                        format_init_flags(unexpected)
                    ))
                })?;
        }
        Ok(snapshot)
    }

    pub(crate) fn log(&self) {
        info!(
            "FOD FUSE compatibility: fuser={} userspace_protocol_max={} kernel_protocol={} negotiated_protocol={} available_capabilities={} fod_requested_capabilities={} fod_enabled_capabilities={} fod_unsupported_capabilities={}",
            FUSER_VERSION,
            USERSPACE_PROTOCOL_MAX,
            self.kernel_protocol,
            self.negotiated_protocol,
            format_init_flags(self.available_capabilities),
            format_init_flags(self.requested_capabilities),
            format_init_flags(self.enabled_capabilities),
            format_init_flags(self.unsupported_capabilities),
        );
        info!(
            "FOD FUSE negotiated: requested_max_write={} effective_max_write={} requested_max_readahead={} effective_max_readahead={} max_background=unavailable congestion_threshold=unavailable",
            self.requested_max_write,
            self.effective_max_write,
            self.requested_max_readahead,
            self.effective_max_readahead,
        );
    }
}

fn requested_limit_from_env(env_name: &str, default_value: u32) -> io::Result<u32> {
    match env_var_with_legacy_alias(env_name) {
        Some(value) if !value.trim().is_empty() => parse_fuse_io_bytes(env_name, &value),
        _ => Ok(default_value),
    }
}

fn parse_fuse_io_bytes(setting_name: &str, value: &str) -> io::Result<u32> {
    let bytes = parse_size_bytes(value).map_err(|err| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("invalid {setting_name}: {err}"),
        )
    })?;
    if bytes == 0 || bytes > u32::MAX as u64 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "invalid {setting_name}: {value}; expected a positive byte size not larger than {} bytes",
                u32::MAX
            ),
        ));
    }
    Ok(bytes as u32)
}

fn apply_limit_with_fallback<F>(
    limit_name: &str,
    requested: u32,
    allow_zero_fallback: bool,
    mut setter: F,
) -> io::Result<u32>
where
    F: FnMut(u32) -> Result<u32, u32>,
{
    match setter(requested) {
        Ok(_) => Ok(requested),
        Err(0) if allow_zero_fallback => {
            warn!(
                "FOD FUSE {} requested={} is unavailable from the kernel; keeping effective=0",
                limit_name, requested
            );
            Ok(0)
        }
        Err(nearest) if nearest > 0 => {
            warn!(
                "FOD FUSE {} requested={} exceeds negotiated limit; retrying with effective={}",
                limit_name, requested, nearest
            );
            setter(nearest).map_err(|retry_nearest| {
                io::Error::other(format!(
                    "fuser rejected fallback {limit_name}={nearest}; nearest reported value={retry_nearest}"
                ))
            })?;
            Ok(nearest)
        }
        Err(_) => Err(io::Error::other(format!(
            "fuser rejected {limit_name}={requested} without a usable fallback"
        ))),
    }
}

fn format_init_flags(flags: InitFlags) -> String {
    if flags.is_empty() {
        return "none".to_string();
    }

    let mut names = flags
        .iter_names()
        .map(|(name, _)| name.strip_prefix("FUSE_").unwrap_or(name).to_string())
        .collect::<Vec<_>>();
    let unknown_bits = flags.bits() & !InitFlags::all().bits();
    if unknown_bits != 0 {
        names.push(format!("UNKNOWN_0x{unknown_bits:016x}"));
    }
    format!("[{}]", names.join(","))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn computes_negotiated_protocol_capabilities_and_io_limits() {
        let available = InitFlags::FUSE_POSIX_LOCKS | InitFlags::FUSE_MAX_PAGES;
        let snapshot = FuseCompatibilitySnapshot::from_parts(
            Version(7, 38),
            available,
            512 * 1024,
            512 * 1024,
            512 * 1024,
            256 * 1024,
        );

        assert_eq!(snapshot.kernel_protocol, Version(7, 38));
        assert_eq!(snapshot.negotiated_protocol, Version(7, 38));
        assert_eq!(snapshot.enabled_capabilities, InitFlags::FUSE_POSIX_LOCKS);
        assert_eq!(
            snapshot.unsupported_capabilities,
            InitFlags::FUSE_FLOCK_LOCKS
        );
        assert_eq!(snapshot.requested_max_write, 512 * 1024);
        assert_eq!(snapshot.effective_max_write, 512 * 1024);
        assert_eq!(snapshot.requested_max_readahead, 512 * 1024);
        assert_eq!(snapshot.effective_max_readahead, 256 * 1024);
        assert_eq!(
            format_init_flags(snapshot.available_capabilities),
            "[POSIX_LOCKS,MAX_PAGES]"
        );
    }

    #[test]
    fn caps_newer_kernel_protocol_at_userspace_maximum() {
        let snapshot = FuseCompatibilitySnapshot::from_parts(
            Version(7, 44),
            InitFlags::empty(),
            DEFAULT_FUSE_MAX_WRITE_BYTES,
            DEFAULT_FUSE_MAX_WRITE_BYTES,
            DEFAULT_FUSE_MAX_READAHEAD_BYTES,
            DEFAULT_FUSE_MAX_READAHEAD_BYTES,
        );

        assert_eq!(snapshot.negotiated_protocol, USERSPACE_PROTOCOL_MAX);
        assert_eq!(format_init_flags(snapshot.enabled_capabilities), "none");
        assert_eq!(
            format_init_flags(InitFlags::from_bits_retain(1_u64 << 63)),
            "[UNKNOWN_0x8000000000000000]"
        );
    }

    #[test]
    fn parses_binary_fuse_io_sizes_and_rejects_invalid_limits() {
        assert_eq!(
            parse_fuse_io_bytes(FUSE_MAX_WRITE_ENV, "512KiB").unwrap(),
            512 * 1024
        );
        assert!(parse_fuse_io_bytes(FUSE_MAX_WRITE_ENV, "0").is_err());
        assert!(parse_fuse_io_bytes(FUSE_MAX_WRITE_ENV, "4GiB").is_err());
    }

    #[test]
    fn retries_with_nearest_supported_limit() {
        let mut applied = 0;
        let effective = apply_limit_with_fallback("max_readahead", 512 * 1024, true, |value| {
            if value > 256 * 1024 {
                Err(256 * 1024)
            } else {
                applied = value;
                Ok(0)
            }
        })
        .unwrap();

        assert_eq!(effective, 256 * 1024);
        assert_eq!(applied, 256 * 1024);
    }

    #[test]
    fn accepts_zero_kernel_readahead_limit_without_failing_mount() {
        let effective =
            apply_limit_with_fallback("max_readahead", 512 * 1024, true, |_| Err(0)).unwrap();
        assert_eq!(effective, 0);
    }

    #[test]
    fn reported_fuser_version_matches_exact_dependency_pin() {
        let manifest = include_str!("../Cargo.toml");
        assert!(manifest.contains(&format!("version = \"={FUSER_VERSION}\"")));
    }
}
