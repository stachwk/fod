// Copyright (c) 2026 Wojciech Stach
// Licensed under BSL 1.1

use fuser::{InitFlags, KernelConfig, Version};
use log::info;
use std::io;

pub(crate) const FUSER_VERSION: &str = "0.17.0";
pub(crate) const USERSPACE_PROTOCOL_MAX: Version = Version(7, 40);

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
}

impl FuseCompatibilitySnapshot {
    fn from_parts(kernel_protocol: Version, available_capabilities: InitFlags) -> Self {
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
        }
    }

    pub(crate) fn configure(config: &mut KernelConfig) -> io::Result<Self> {
        let snapshot = Self::from_parts(config.kernel_abi(), config.capabilities());
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
            "FOD FUSE compatibility: fuser={} userspace_protocol_max={} kernel_protocol={} negotiated_protocol={} available_capabilities={} fod_requested_capabilities={} fod_enabled_capabilities={} fod_unsupported_capabilities={} max_write=unavailable max_readahead=unavailable max_background=unavailable congestion_threshold=unavailable",
            FUSER_VERSION,
            USERSPACE_PROTOCOL_MAX,
            self.kernel_protocol,
            self.negotiated_protocol,
            format_init_flags(self.available_capabilities),
            format_init_flags(self.requested_capabilities),
            format_init_flags(self.enabled_capabilities),
            format_init_flags(self.unsupported_capabilities),
        );
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
    fn computes_negotiated_protocol_and_capability_subsets() {
        let available = InitFlags::FUSE_POSIX_LOCKS | InitFlags::FUSE_MAX_PAGES;
        let snapshot = FuseCompatibilitySnapshot::from_parts(Version(7, 38), available);

        assert_eq!(snapshot.kernel_protocol, Version(7, 38));
        assert_eq!(snapshot.negotiated_protocol, Version(7, 38));
        assert_eq!(snapshot.enabled_capabilities, InitFlags::FUSE_POSIX_LOCKS);
        assert_eq!(
            snapshot.unsupported_capabilities,
            InitFlags::FUSE_FLOCK_LOCKS
        );
        assert_eq!(
            format_init_flags(snapshot.available_capabilities),
            "[POSIX_LOCKS,MAX_PAGES]"
        );
    }

    #[test]
    fn caps_newer_kernel_protocol_at_userspace_maximum() {
        let snapshot = FuseCompatibilitySnapshot::from_parts(Version(7, 44), InitFlags::empty());

        assert_eq!(snapshot.negotiated_protocol, USERSPACE_PROTOCOL_MAX);
        assert_eq!(format_init_flags(snapshot.enabled_capabilities), "none");
        assert_eq!(
            format_init_flags(InitFlags::from_bits_retain(1_u64 << 63)),
            "[UNKNOWN_0x8000000000000000]"
        );
    }

    #[test]
    fn reported_fuser_version_matches_exact_dependency_pin() {
        let manifest = include_str!("../Cargo.toml");
        assert!(manifest.contains(&format!("version = \"={FUSER_VERSION}\"")));
    }
}
